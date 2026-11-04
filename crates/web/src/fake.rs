//! W-3 fake modes: local stand-ins for every outside service, so a judge
//! or developer can run the whole product with no accounts and no money
//! moving (`docs/specs/config-db.md` § Fake mode).
//!
//! The `<area>.mode` rows live in the config crate ([`config::Modes`]);
//! its loader refuses a fake on any boot whose binds are not all
//! loopback, unless the host is a labelled staging target that is not
//! production. This module holds the web side:
//!
//! - the config writers' matching refusal, so a production console can
//!   never save a row that would stop its own next boot;
//! - the fake **outbox** (`mailer.mode = fake`): [`OutboxMailer`] keeps
//!   every outgoing email (magic links, follower digests, outreach) in
//!   the `fake_outbox` table instead of sending it, and staff read them
//!   at `GET /admin/fake/outbox`. Nothing is sent, so nothing reaches a
//!   real address and the overall send budget is untouched. The rows
//!   hold live magic links: the outbox exists only on loopback and
//!   staging boots, where the data is synthetic;
//! - the fake **inbox** (`mail.mode = fake`): `POST /admin/fake/inbox`
//!   (the Compose screen) builds the message Gmail would deliver and
//!   captures it through the Gmail poll's own capture step
//!   ([`mail::fake`]), so the DMARC trust gate, threading and the
//!   instruct worker → `/review` loop run unchanged.

use axum::{
    Json, Router,
    extract::{DefaultBodyLimit, State},
    http::StatusCode,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::Engine as _;
use handbook_store::{db, store::Store};
use rusqlite::{Connection, OptionalExtension};
use serde::Deserialize;
use serde_json::json;

use crate::{AppState, admin::AdminAuth, store_unavailable, unix_now};
use auth::magic as flow;

/// Outbox rows the list returns (newest first).
const OUTBOX_LIST_MAX: i64 = 200;
/// Compose body cap: the 8 MiB message plus base64 and JSON overhead.
const COMPOSE_BODY_BYTES: usize = 12 << 20;
/// `To` of composed mail when no Gmail mailbox was ever polled.
const FAKE_MAILBOX: &str = "inbox@fake-inbox.invalid";

/// Lane DDL (`CREATE TABLE IF NOT EXISTS` only; no numbered migration).
pub fn migrate_tables(conn: &Connection) -> rusqlite::Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS fake_outbox (
            id INTEGER PRIMARY KEY,
            at INTEGER NOT NULL,
            kind TEXT NOT NULL,
            to_addr TEXT NOT NULL,
            subject TEXT NOT NULL,
            headers TEXT NOT NULL,
            body TEXT NOT NULL
        );",
    )
}

/// The fake outbox (`mailer.mode = fake`): every send becomes a
/// `fake_outbox` row. Never dials anything.
#[derive(Clone)]
pub struct OutboxMailer {
    pub store_root: std::path::PathBuf,
}

impl OutboxMailer {
    /// Keep one message: the closed header set the real mailers send
    /// (RFC 8058 pair, `Message-ID`, `Reply-To`) rides along as JSON.
    pub fn send_with_headers(
        &self,
        to: &str,
        subject: &str,
        text_body: &str,
        headers: &[(&str, &str)],
    ) -> Result<(), flow::MailError> {
        let unavailable = || flow::MailError::Transport("fake outbox unavailable".into());
        let conn = db::open(&Store::new(&self.store_root)).map_err(|_| unavailable())?;
        migrate_tables(&conn).map_err(|_| unavailable())?;
        let mut kept = serde_json::Map::new();
        for (name, value) in headers {
            for known in [
                "List-Unsubscribe",
                "List-Unsubscribe-Post",
                "Message-ID",
                "Reply-To",
            ] {
                if name.eq_ignore_ascii_case(known) {
                    kept.insert(known.to_string(), json!(value));
                }
            }
        }
        conn.execute(
            "INSERT INTO fake_outbox(at, kind, to_addr, subject, headers, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            rusqlite::params![
                unix_now(),
                crate::magic::send_kind(headers),
                to,
                subject,
                serde_json::Value::Object(kept).to_string(),
                text_body
            ],
        )
        .map_err(|_| unavailable())?;
        Ok(())
    }
}

impl flow::Mailer for OutboxMailer {
    fn send(&self, to: &str, subject: &str, text_body: &str) -> Result<(), flow::MailError> {
        self.send_with_headers(to, subject, text_body, &[])
    }
}

/// Staff routes (the admin guard stack wraps them in `main`).
pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/fake/outbox", get(list_outbox))
        .route(
            "/apostille/api/v1/admin/fake/inbox",
            post(compose).layer(DefaultBodyLimit::max(COMPOSE_BODY_BYTES)),
        )
        .with_state(state)
}

/// `GET /admin/fake/outbox`: the newest kept messages, plus whether
/// the outbox and the Compose inbox are on (`mailer.mode` / `mail.mode`).
async fn list_outbox(_auth: AdminAuth, State(state): State<AppState>) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return store_unavailable();
    };
    if migrate_tables(&conn).is_err() {
        return store_unavailable();
    }
    let rows = conn
        .prepare(
            "SELECT id, at, kind, to_addr, subject, headers, body FROM fake_outbox
             ORDER BY id DESC LIMIT ?1",
        )
        .and_then(|mut stmt| {
            stmt.query_map([OUTBOX_LIST_MAX], |r| {
                let headers: String = r.get(5)?;
                Ok(json!({
                    "id": r.get::<_, i64>(0)?,
                    "at": r.get::<_, i64>(1)?,
                    "kind": r.get::<_, String>(2)?,
                    "to": r.get::<_, String>(3)?,
                    "subject": r.get::<_, String>(4)?,
                    "headers": serde_json::from_str::<serde_json::Value>(&headers)
                        .unwrap_or_else(|_| json!({})),
                    "body": r.get::<_, String>(6)?,
                }))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()
        });
    let Ok(messages) = rows else {
        return store_unavailable();
    };
    Json(json!({
        "outbox": state.fakes.is_fake(config::Integration::MailOut),
        "inbox": state.fakes.is_fake(config::Integration::MailIn),
        "messages": messages,
    }))
    .into_response()
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComposeBody {
    from: String,
    subject: String,
    body: String,
    #[serde(default)]
    in_reply_to: Option<String>,
    /// `pass` or `fail`: what Gmail's stamp reports for the From domain.
    dmarc: String,
    #[serde(default)]
    attachments: Vec<ComposeAttachment>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ComposeAttachment {
    name: String,
    content_type: String,
    data_base64: String,
}

fn unprocessable(why: &str) -> Response {
    (
        StatusCode::UNPROCESSABLE_ENTITY,
        Json(json!({"error": why})),
    )
        .into_response()
}

/// `POST /admin/fake/inbox` (Compose): capture one written message as
/// if the Gmail poll had fetched it. 409 unless `mail.mode = fake`.
async fn compose(
    auth: AdminAuth,
    State(state): State<AppState>,
    Json(body): Json<ComposeBody>,
) -> Response {
    if !state.fakes.is_fake(config::Integration::MailIn) {
        return (
            StatusCode::CONFLICT,
            Json(json!({"error": "incoming email is not in fake mode (mail.mode)"})),
        )
            .into_response();
    }
    let dmarc_pass = match body.dmarc.as_str() {
        "pass" => true,
        "fail" => false,
        _ => return unprocessable("dmarc: want pass or fail"),
    };
    let mut attachments = Vec::with_capacity(body.attachments.len());
    for a in body.attachments {
        let Ok(bytes) = base64::engine::general_purpose::STANDARD.decode(a.data_base64.trim())
        else {
            return unprocessable("attachment data_base64 is not base64");
        };
        attachments.push(mail::fake::FakeAttachment {
            name: a.name,
            content_type: a.content_type,
            bytes,
        });
    }
    let in_reply_to = body
        .in_reply_to
        .map(|id| id.trim().to_string())
        .filter(|id| !id.is_empty());
    let root = state.store_root.clone();
    let actor = auth.user.user_id.clone();
    let from = body.from.trim().to_ascii_lowercase();
    let (subject, text) = (body.subject, body.body);
    let done = tokio::task::spawn_blocking(move || {
        let store = Store::new(&root);
        let conn = db::open(&store).map_err(|e| mail::fake::FakeError::Capture(e.to_string()))?;
        let polled: Option<String> = conn
            .query_row("SELECT email FROM mail_poll WHERE id = 1", [], |r| r.get(0))
            .optional()
            .ok()
            .flatten()
            .filter(|m: &String| !m.trim().is_empty());
        let now = unix_now();
        let mail = mail::fake::FakeMail {
            id: uuid::Uuid::new_v4().simple().to_string(),
            from,
            to: polled.unwrap_or_else(|| FAKE_MAILBOX.to_string()),
            subject,
            body: text,
            in_reply_to,
            dmarc_pass,
            attachments,
            date: now,
        };
        let opts = mail::poll::PollOpts {
            actor,
            now,
            backoff_secs: Vec::new(),
            max_fetches: 1,
        };
        let captured = mail::fake::ingest_fake(&conn, &store, &opts, &mail)?;
        let slug = match &captured {
            mail::poll::Captured::Created { capture_id, .. } => conn
                .query_row(
                    "SELECT a.slug FROM capture c JOIN authority a ON a.id = c.authority_id
                     WHERE c.id = ?1",
                    [capture_id],
                    |r| r.get::<_, String>(0),
                )
                .ok(),
            _ => None,
        };
        Ok::<_, mail::fake::FakeError>((captured, slug))
    })
    .await;
    match done {
        Ok(Ok((
            mail::poll::Captured::Created {
                capture_id,
                trust,
                verified_by,
                outreach_email,
            },
            slug,
        ))) => (
            StatusCode::CREATED,
            Json(json!({
                "captured": true,
                "capture_id": capture_id,
                "trust": trust,
                "verified_by": verified_by,
                "authority": slug,
                "outreach_email": outreach_email,
            })),
        )
            .into_response(),
        Ok(Ok((mail::poll::Captured::Duplicate, _))) => {
            Json(json!({"captured": false, "duplicate": true})).into_response()
        }
        Ok(Ok((mail::poll::Captured::Report { suppressed }, _))) => {
            Json(json!({"captured": false, "suppressed": suppressed})).into_response()
        }
        Ok(Err(mail::fake::FakeError::Invalid(why))) => unprocessable(why),
        Ok(Err(mail::fake::FakeError::Capture(_))) | Err(_) => store_unavailable(),
    }
}

/// 422 when `key` is a `<area>.mode` row, `value` asks for `fake`, and
/// this boot's guard is closed; `None` lets the write through.
pub(crate) fn refuse_fake_row(
    modes: &config::Modes,
    key: &str,
    value: &serde_json::Value,
) -> Option<Response> {
    let mode_row = config::Integration::ALL.iter().any(|i| i.key() == key);
    let asks_fake = value.as_str().and_then(config::Mode::parse) == Some(config::Mode::Fake);
    (mode_row && asks_fake && !modes.fakes_allowed).then(|| {
        (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "fake mode runs only on loopback binds or a staging target"})),
        )
            .into_response()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outbox_keeps_every_kind_of_mail_and_sends_none() {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let outbox = OutboxMailer {
            store_root: tmp.path().to_path_buf(),
        };
        // A magic link (plain send), a digest (RFC 8058 pair) and an
        // unknown header that must not ride along.
        flow::Mailer::send(&outbox, "a@example.test", "Sign in", "https://x.test/l#t").unwrap();
        outbox
            .send_with_headers(
                "b@example.test",
                "Digest",
                "news",
                &[
                    ("List-Unsubscribe", "<https://x.test/u>"),
                    ("List-Unsubscribe-Post", "List-Unsubscribe=One-Click"),
                    ("X-Other", "dropped"),
                ],
            )
            .unwrap();
        let conn = db::open(&store).unwrap();
        let rows: Vec<(String, String, String)> = conn
            .prepare("SELECT kind, to_addr, headers FROM fake_outbox ORDER BY id")
            .unwrap()
            .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))
            .unwrap()
            .collect::<rusqlite::Result<_>>()
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            (rows[0].0.as_str(), rows[0].1.as_str()),
            ("mail", "a@example.test")
        );
        assert_eq!(rows[1].0, "digest");
        let headers: serde_json::Value = serde_json::from_str(&rows[1].2).unwrap();
        assert_eq!(headers["List-Unsubscribe"], "<https://x.test/u>");
        assert!(headers.get("X-Other").is_none());
        // Nothing claimed the overall send budget.
        let sent: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE name = 'mail_send_log'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(sent, 0);
    }

    #[test]
    fn only_a_fake_mode_row_on_a_closed_guard_is_refused() {
        let closed = config::Modes::default();
        let mut open = config::Modes::default();
        open.fakes_allowed = true;
        let fake = json!("fake");
        assert!(refuse_fake_row(&closed, "labels.mode", &fake).is_some());
        assert!(refuse_fake_row(&closed, "mailer.mode", &json!(" fake ")).is_some());
        assert!(refuse_fake_row(&closed, "labels.mode", &json!("live")).is_none());
        assert!(refuse_fake_row(&closed, "mailer.host", &fake).is_none());
        assert!(refuse_fake_row(&open, "labels.mode", &fake).is_none());
    }
}
