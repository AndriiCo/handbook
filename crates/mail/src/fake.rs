//! W-3 fake inbox: mail a person writes in the staff console's Compose
//! screen, captured as if the Gmail poll had fetched it.
//!
//! [`fake_message`] builds the RFC 2822 bytes Gmail would hand the poll,
//! including the delivery stamp Gmail writes (`Authentication-Results:
//! mx.google.com; …`) with the DMARC result the composer chose. The
//! bytes then go through [`crate::poll::capture_message`] — the poll's
//! own capture step — so sender resolution, the DMARC/DKIM trust gate,
//! bounce handling, outreach threading and the sidecar are the code a
//! real message meets, and the instruct worker → `/review` loop reads
//! the capture unchanged. A DMARC pass from an official domain earns
//! `official` trust exactly as it would from Gmail; a fail caps it at
//! `community`.
//!
//! Watermarks: an `X-Handbook-Fake` header in the blob, a
//! `fake-inbox:<id>` capture origin, and `received_via: fake-inbox` +
//! `fake: true` in the authenticity record. The web crate offers
//! Compose only while `mail.mode = fake`, which the config loader
//! allows on loopback or staging boots alone.

use base64::Engine as _;
use rusqlite::Connection;

use handbook_store::store::Store;

use crate::client::MailError;
use crate::poll::{Captured, PollOpts, capture_message};

/// Largest composed message (body plus attachments), in bytes.
pub const MAX_FAKE_BYTES: usize = 8 << 20;

/// One attachment on a composed message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeAttachment {
    pub name: String,
    /// `type/subtype` (e.g. `application/pdf`, `text/plain`).
    pub content_type: String,
    pub bytes: Vec<u8>,
}

/// One message written in the Compose screen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FakeMail {
    /// Unique id (the caller mints it): the Message-ID and the capture
    /// origin carry it.
    pub id: String,
    pub from: String,
    pub to: String,
    pub subject: String,
    pub body: String,
    /// A `<message-id>` this answers (an outbox message), threading the
    /// reply to its outreach email.
    pub in_reply_to: Option<String>,
    /// The DMARC result Gmail's stamp reports for the From domain.
    pub dmarc_pass: bool,
    pub attachments: Vec<FakeAttachment>,
    /// Unix seconds for the `Date` header.
    pub date: i64,
}

/// Why a composed message was refused before capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FakeError {
    /// A field is malformed (the message names it).
    Invalid(&'static str),
    /// Capture failed (store error).
    Capture(String),
}

impl std::fmt::Display for FakeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FakeError::Invalid(why) => write!(f, "{why}"),
            FakeError::Capture(why) => write!(f, "capture failed: {why}"),
        }
    }
}

impl From<MailError> for FakeError {
    fn from(e: MailError) -> Self {
        FakeError::Capture(e.to_string())
    }
}

/// A header value: printable, one line (no CR/LF injection).
fn one_line(value: &str) -> bool {
    !value.chars().any(|c| c.is_control())
}

/// `local@domain.tld` with nothing a header could misread.
fn plain_address(value: &str) -> bool {
    let Some((local, domain)) = value.split_once('@') else {
        return false;
    };
    !local.is_empty()
        && domain.contains('.')
        && !domain.starts_with('.')
        && !domain.ends_with('.')
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"@._+-".contains(&b))
}

/// A `<message-id>` in angle brackets.
fn message_id(value: &str) -> bool {
    value.len() <= 300
        && value.starts_with('<')
        && value.ends_with('>')
        && value.contains('@')
        && value[1..value.len() - 1]
            .bytes()
            .all(|b| b.is_ascii_graphic() && b != b'<' && b != b'>')
}

/// `type/subtype` token pair.
fn content_type(value: &str) -> bool {
    let token = |t: &str| {
        !t.is_empty()
            && t.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"!#$&-^_.+".contains(&b))
    };
    value
        .split_once('/')
        .is_some_and(|(kind, sub)| token(kind) && token(sub))
}

fn crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}

/// RFC 2822 date for unix seconds (UTC).
fn rfc2822(unix: i64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    // Civil-from-days (Howard Hinnant), proleptic Gregorian.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} +0000",
        DAYS[days.rem_euclid(7) as usize],
        day,
        MONTHS[(month - 1) as usize],
        year,
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// Build the raw message Gmail would hand the poll. Refuses header
/// injection, malformed addresses and oversized mail.
pub fn fake_message(mail: &FakeMail) -> Result<Vec<u8>, FakeError> {
    if !plain_address(&mail.from) {
        return Err(FakeError::Invalid(
            "from: want one plain address (name@domain.tld)",
        ));
    }
    if !plain_address(&mail.to) {
        return Err(FakeError::Invalid("to: want one plain address"));
    }
    if !one_line(&mail.subject) || mail.subject.len() > 500 {
        return Err(FakeError::Invalid("subject: one line, at most 500 bytes"));
    }
    if mail.id.is_empty()
        || !mail
            .id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    {
        return Err(FakeError::Invalid("id: letters, digits and dashes"));
    }
    if let Some(id) = &mail.in_reply_to
        && !message_id(id)
    {
        return Err(FakeError::Invalid("in_reply_to: want one <message-id>"));
    }
    let size: usize = mail.body.len()
        + mail
            .attachments
            .iter()
            .map(|a| a.bytes.len())
            .sum::<usize>();
    if size > MAX_FAKE_BYTES {
        return Err(FakeError::Invalid(
            "message: at most 8 MiB with attachments",
        ));
    }
    for attachment in &mail.attachments {
        if attachment.name.trim().is_empty()
            || attachment.name.len() > 200
            || !one_line(&attachment.name)
            || attachment.name.contains(['"', '\\'])
        {
            return Err(FakeError::Invalid(
                "attachment name: one line, no quotes, at most 200 bytes",
            ));
        }
        if !content_type(&attachment.content_type) {
            return Err(FakeError::Invalid(
                "attachment content_type: want type/subtype",
            ));
        }
    }
    let domain = mail.from.rsplit_once('@').map_or("", |(_, d)| d);
    let verdict = if mail.dmarc_pass { "pass" } else { "fail" };
    let mut head = String::new();
    // Gmail's delivery stamp, topmost: the one the trust gate reads.
    head.push_str(&format!(
        "Authentication-Results: mx.google.com; dkim={verdict} header.d={domain}; spf={verdict} smtp.mailfrom={from}; dmarc={verdict} header.from={domain}\r\n",
        from = mail.from
    ));
    head.push_str(
        "X-Handbook-Fake: composed in the staff console (W-3 fake inbox); no mail server delivered it\r\n",
    );
    head.push_str(&format!("From: {}\r\n", mail.from));
    head.push_str(&format!("To: {}\r\n", mail.to));
    head.push_str(&format!("Date: {}\r\n", rfc2822(mail.date)));
    head.push_str(&format!("Subject: {}\r\n", mail.subject));
    head.push_str(&format!(
        "Message-ID: <fake-{}@fake-inbox.invalid>\r\n",
        mail.id
    ));
    if let Some(id) = &mail.in_reply_to {
        head.push_str(&format!("In-Reply-To: {id}\r\nReferences: {id}\r\n"));
    }
    head.push_str("MIME-Version: 1.0\r\n");
    let text_part = format!(
        "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n{}\r\n",
        crlf(&mail.body)
    );
    let mut out = head;
    if mail.attachments.is_empty() {
        out.push_str(&text_part);
    } else {
        let boundary = format!("fake-{}", mail.id);
        out.push_str(&format!(
            "Content-Type: multipart/mixed; boundary=\"{boundary}\"\r\n\r\n"
        ));
        out.push_str(&format!("--{boundary}\r\n{text_part}"));
        for attachment in &mail.attachments {
            let encoded = base64::engine::general_purpose::STANDARD.encode(&attachment.bytes);
            out.push_str(&format!(
                "--{boundary}\r\nContent-Type: {ct}; name=\"{name}\"\r\nContent-Disposition: attachment; filename=\"{name}\"\r\nContent-Transfer-Encoding: base64\r\n\r\n",
                ct = attachment.content_type,
                name = attachment.name
            ));
            for line in encoded.as_bytes().chunks(76) {
                out.push_str(std::str::from_utf8(line).unwrap_or_default());
                out.push_str("\r\n");
            }
        }
        out.push_str(&format!("--{boundary}--\r\n"));
    }
    Ok(out.into_bytes())
}

/// Compose and capture one fake message through the poll's capture
/// step ([`capture_message`]). See the module docs.
pub fn ingest_fake(
    conn: &Connection,
    store: &Store,
    opts: &PollOpts,
    mail: &FakeMail,
) -> Result<Captured, FakeError> {
    let raw = fake_message(mail)?;
    let parsed = crate::mime::parse(&raw);
    let provenance = serde_json::json!({
        "fake": true,
        "fake_id": mail.id,
        "received_via": "fake-inbox",
    });
    Ok(capture_message(
        conn,
        store,
        opts,
        &raw,
        &parsed,
        &format!("fake-inbox:{}", mail.id),
        provenance,
    )?)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mail(from: &str, dmarc_pass: bool) -> FakeMail {
        FakeMail {
            id: "f1".to_string(),
            from: from.to_string(),
            to: "inbox@fake-inbox.invalid".to_string(),
            subject: "Office closure tomorrow".to_string(),
            body: "The Olympia office is closed tomorrow.\nCall ahead.".to_string(),
            in_reply_to: None,
            dmarc_pass,
            attachments: Vec::new(),
            date: 1_988_150_400,
        }
    }

    fn migrated() -> (tempfile::TempDir, Store, Connection) {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        (dir, store, conn)
    }

    fn opts() -> PollOpts {
        PollOpts {
            actor: "staff-1".to_string(),
            now: 1_988_150_400,
            backoff_secs: Vec::new(),
            max_fetches: 10,
        }
    }

    /// An official `example.gov` authority (the poll tests' shape).
    fn seed_official(conn: &Connection) -> i64 {
        let id = handbook_store::db::add_authority(
            conn,
            &handbook_store::db::NewAuthority {
                kind: "office",
                country: "USA",
                region: "TS",
                name: "Test SOS",
                slug: "t-sos",
                actor: "t",
                at: 1,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO domain(name, authority_id, status) VALUES ('example.gov', ?1, 'official')",
            [id],
        )
        .unwrap();
        id
    }

    fn capture(conn: &Connection, id: i64) -> (String, String, String, serde_json::Value) {
        let (trust, slug, authenticity): (String, String, String) = conn
            .query_row(
                "SELECT c.trust, a.slug, c.authenticity FROM capture c
                 JOIN authority a ON a.id = c.authority_id WHERE c.id = ?1",
                [id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .unwrap();
        // The capture's origin lands in its ingest audit row.
        let origin: String = conn
            .query_row(
                "SELECT reason FROM audit WHERE action = 'ingest_capture' AND entity_id = ?1",
                [id],
                |r| r.get(0),
            )
            .unwrap();
        (
            trust,
            slug,
            origin,
            serde_json::from_str(&authenticity).unwrap(),
        )
    }

    #[test]
    fn dmarc_decides_trust_exactly_as_for_gmail_mail() {
        let (_dir, store, conn) = migrated();
        seed_official(&conn);
        // DMARC pass from the official domain: official trust, the
        // office's own authority, watermarked as fake.
        let Captured::Created {
            capture_id,
            trust,
            verified_by,
            outreach_email,
        } = ingest_fake(&conn, &store, &opts(), &mail("ops@example.gov", true)).unwrap()
        else {
            panic!("want a capture");
        };
        assert_eq!(
            (trust, verified_by, outreach_email),
            ("official", "dmarc", None)
        );
        let (trust, slug, origin, evidence) = capture(&conn, capture_id);
        assert_eq!((trust.as_str(), slug.as_str()), ("official", "t-sos"));
        assert!(origin.starts_with("fetch fake-inbox:f1 "), "{origin}");
        assert_eq!(evidence["received_via"], "fake-inbox");
        assert_eq!(evidence["fake"], true);
        assert_eq!(evidence["auth_results"]["dmarc"], "pass");
        assert!(evidence.get("gmail_id").is_none());
        // DMARC fail: kept on the office, capped at community.
        let mut failed = mail("ops@example.gov", false);
        failed.id = "f2".to_string();
        let Captured::Created {
            capture_id,
            trust,
            verified_by,
            ..
        } = ingest_fake(&conn, &store, &opts(), &failed).unwrap()
        else {
            panic!("want a capture");
        };
        assert_eq!((trust, verified_by), ("community", "none"));
        let (_, slug, _, evidence) = capture(&conn, capture_id);
        assert_eq!(slug, "t-sos");
        assert_eq!(evidence["auth_results"]["dmarc"], "fail");
        // An unknown domain with a DMARC pass lands in the inbox as
        // community, like any polled stranger.
        let mut stranger = mail("someone@unknown.test", true);
        stranger.id = "f3".to_string();
        let Captured::Created {
            capture_id, trust, ..
        } = ingest_fake(&conn, &store, &opts(), &stranger).unwrap()
        else {
            panic!("want a capture");
        };
        assert_eq!(trust, "community");
        assert_eq!(capture(&conn, capture_id).1, "mail-inbox");
        // The sidecar the instruct worker reads: domain, never address.
        let text = std::fs::read_dir(store.root().join("text")).map(|d| d.count());
        assert!(text.is_ok_and(|n| n >= 3));
    }

    #[test]
    fn a_reply_threads_to_the_outreach_email_it_answers() {
        let (_dir, store, conn) = migrated();
        seed_official(&conn);
        conn.execute_batch(
            "CREATE TABLE outreach_email (id INTEGER PRIMARY KEY, message_id TEXT,
             status TEXT, reply_capture_id INTEGER);
             INSERT INTO outreach_email(id, message_id, status)
             VALUES (7, '<outreach-7-ab@andrii.ai>', 'sent');",
        )
        .unwrap();
        let mut reply = mail("ops@example.gov", true);
        reply.in_reply_to = Some("<outreach-7-ab@andrii.ai>".to_string());
        let Captured::Created {
            capture_id,
            outreach_email,
            ..
        } = ingest_fake(&conn, &store, &opts(), &reply).unwrap()
        else {
            panic!("want a capture");
        };
        assert_eq!(outreach_email, Some(7));
        let (status, linked): (String, i64) = conn
            .query_row(
                "SELECT status, reply_capture_id FROM outreach_email WHERE id = 7",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!((status.as_str(), linked), ("replied", capture_id));
        // Composing the identical message twice is a duplicate.
        assert_eq!(
            ingest_fake(&conn, &store, &opts(), &reply).unwrap(),
            Captured::Duplicate
        );
    }

    #[test]
    fn attachments_reach_the_sidecar() {
        let mut with_file = mail("ops@example.gov", true);
        with_file.attachments.push(FakeAttachment {
            name: "hours.txt".to_string(),
            content_type: "text/plain".to_string(),
            bytes: b"Open 8-4 Mon-Thu.".to_vec(),
        });
        let raw = fake_message(&with_file).unwrap();
        let parsed = crate::mime::parse(&raw);
        assert!(
            parsed.body_text.contains("Open 8-4 Mon-Thu."),
            "{}",
            parsed.body_text
        );
        assert!(parsed.body_text.contains("Call ahead."));
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.subject, "Office closure tomorrow");
        assert_eq!(parsed.date, "Sat, 01 Jan 2033 00:00:00 +0000");
    }

    #[test]
    fn header_injection_and_bad_fields_are_refused() {
        let base = mail("ops@example.gov", true);
        let cases: Vec<fn(&mut FakeMail)> = vec![
            |m| m.from = "ops@example.gov\r\nBcc: x@y.test".to_string(),
            |m| m.from = "Ops <ops@example.gov>".to_string(),
            |m| m.subject = "hi\r\nX-Evil: 1".to_string(),
            |m| m.in_reply_to = Some("<a@b>\r\nX: y".to_string()),
            |m| m.in_reply_to = Some("no-brackets@x".to_string()),
            |m| m.id = "../x".to_string(),
            |m| {
                m.attachments.push(FakeAttachment {
                    name: "a\".pdf".to_string(),
                    content_type: "application/pdf".to_string(),
                    bytes: vec![],
                })
            },
            |m| {
                m.attachments.push(FakeAttachment {
                    name: "a.pdf".to_string(),
                    content_type: "application/pdf\r\nX: y".to_string(),
                    bytes: vec![],
                })
            },
            |m| m.body = "x".repeat(MAX_FAKE_BYTES + 1),
        ];
        for (i, edit) in cases.iter().enumerate() {
            let mut m = base.clone();
            edit(&mut m);
            assert!(
                matches!(fake_message(&m), Err(FakeError::Invalid(_))),
                "case {i} accepted"
            );
        }
        assert!(fake_message(&base).is_ok());
    }
}
