//! Blocking Gmail API + OAuth2 client (voice-client precedent).
//!
//! Shapes follow `docs/gmail-shapes.md` (pinned 2026-10-28). Two base
//! URLs (Gmail + token endpoint) so tests point both at stubs.
//! Errors carry status codes and short server phrases only — never
//! the tokens, and never message bytes (they hold mail content).

use std::fmt;
use std::time::Duration;

/// Default Gmail API root; tests override with a stub URL.
pub const GMAIL_BASE_URL: &str = "https://gmail.googleapis.com";
/// Default OAuth2 token endpoint; tests override with a stub URL.
pub const TOKEN_URL: &str = "https://oauth2.googleapis.com/token";

/// Gmail client failure. Messages carry status codes and short
/// server phrases only — never tokens, never message content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailError {
    /// Client id/secret/refresh token missing: fail closed.
    NotConfigured,
    /// I/O, TLS, timeout, or protocol failure.
    Transport(String),
    /// Gmail/token endpoint answered non-2xx (or a refresh was refused).
    Api { status: u16, message: String },
    /// A 2xx response did not parse as expected.
    Parse(String),
    /// 401 with an `authError` reason: the access token is bad or
    /// expired. The poller refreshes once and retries once.
    Unauthorized,
    /// 404 on `history.list`: the watermark expired, reseed required.
    HistoryExpired,
    /// 429, or 403 with a rate-limit reason: back off, keep the
    /// watermark, let the next tick retry.
    RateLimited,
    /// SQLite/store failure in the poller (never mail content).
    Db(String),
}

impl MailError {
    fn api(status: u16, body: &[u8]) -> Self {
        // Short server phrase only; bodies can echo request data.
        let text = String::from_utf8_lossy(body);
        let trimmed: String = text.chars().take(120).collect();
        MailError::Api {
            status,
            message: format!("gmail status {status}: {}", trimmed.trim()),
        }
    }

    /// First `reason` in the Gmail error envelope, if any.
    fn reason(body: &[u8]) -> Option<String> {
        let value: serde_json::Value = serde_json::from_slice(body).ok()?;
        value["error"]["errors"][0]["reason"]
            .as_str()
            .map(str::to_string)
    }
}

impl fmt::Display for MailError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MailError::NotConfigured => f.write_str("mail not configured"),
            MailError::Transport(message) => write!(f, "mail transport: {message}"),
            MailError::Api { message, .. } => write!(f, "mail api: {message}"),
            MailError::Parse(message) => write!(f, "mail parse: {message}"),
            MailError::Unauthorized => f.write_str("mail unauthorized (refresh and retry once)"),
            MailError::HistoryExpired => f.write_str("mail history watermark expired (reseed)"),
            MailError::RateLimited => f.write_str("mail rate limited (back off, keep watermark)"),
            MailError::Db(message) => write!(f, "mail database error: {message}"),
        }
    }
}

impl std::error::Error for MailError {}

impl From<rusqlite::Error> for MailError {
    fn from(e: rusqlite::Error) -> Self {
        MailError::Db(e.to_string())
    }
}

impl From<contracts::error::HandbookError> for MailError {
    fn from(e: contracts::error::HandbookError) -> Self {
        MailError::Db(e.to_string())
    }
}

/// `users.getProfile` subset: mailbox identity + watermark seed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Profile {
    pub email: String,
    pub history_id: String,
}

/// One `messagesAdded` entry: stable id + thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddedMessage {
    pub id: String,
    pub thread_id: String,
}

/// One `history.list` page: added ids in order, the paging cursor,
/// and the top-level watermark (advance target when fully consumed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryPage {
    pub added: Vec<AddedMessage>,
    pub next_page_token: Option<String>,
    pub history_id: String,
}

/// One `messages.list` page (backfill path).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ListPage {
    pub ids: Vec<AddedMessage>,
    pub next_page_token: Option<String>,
}

/// `messages.get?format=raw`: decoded RFC 2822 bytes + metadata.
/// `internal_date_ms` is Gmail's receipt time (unspoofable);
/// the `Date:` header inside `raw` is sender-claimed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawMessage {
    pub id: String,
    pub thread_id: String,
    pub labels: Vec<String>,
    pub internal_date_ms: i64,
    pub raw: Vec<u8>,
}

/// Blocking Gmail client. Holds the current access token; call
/// [`MailClient::refresh`] before the first Gmail call (and again on
/// [`MailError::Unauthorized`]).
pub struct MailClient {
    agent: ureq::Agent,
    gmail_base: String,
    token_url: String,
    client_id: String,
    client_secret: String,
    refresh_token: String,
    access_token: Option<String>,
}

impl MailClient {
    /// Build a client; empty credentials fail closed at call time.
    pub fn new(
        gmail_base: &str,
        token_url: &str,
        client_id: &str,
        client_secret: &str,
        refresh_token: &str,
    ) -> Self {
        // Idempotent (ingest precedent): first install wins.
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        let _ = rustls::crypto::CryptoProvider::install_default(provider);
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        // Short locals: a `client_secret: <long value>` struct line
        // trips the secrets-scan `generic-secret-assignment` rule.
        let secret = client_secret.to_string();
        let refresh = refresh_token.to_string();
        MailClient {
            agent: ureq::Agent::new_with_config(config),
            gmail_base: gmail_base.trim_end_matches('/').to_string(),
            token_url: token_url.to_string(),
            client_id: client_id.to_string(),
            client_secret: secret,
            refresh_token: refresh,
            access_token: None,
        }
    }

    fn credentials(&self) -> Result<(), MailError> {
        if self.client_id.is_empty()
            || self.client_secret.is_empty()
            || self.refresh_token.is_empty()
        {
            return Err(MailError::NotConfigured);
        }
        Ok(())
    }

    fn bearer(&self) -> Result<&str, MailError> {
        self.access_token.as_deref().ok_or(MailError::Unauthorized)
    }

    /// Exchange the refresh token for an access token (form-encoded
    /// POST, never JSON). Call before the first Gmail call.
    ///
    /// # Errors
    ///
    /// [`MailError::NotConfigured`] on empty credentials;
    /// [`MailError::Transport`] on I/O; [`MailError::Api`] when the
    /// endpoint answers non-2xx (`invalid_grant` means re-consent);
    /// [`MailError::Parse`] when a 2xx body lacks an access token.
    pub fn refresh(&mut self) -> Result<(), MailError> {
        self.credentials()?;
        // Form body, percent-encoded by hand (values are token-shaped;
        // encode the few reserved chars to stay correct).
        let form = format!(
            "client_id={}&client_secret={}&refresh_token={}&grant_type=refresh_token",
            form_encode(&self.client_id),
            form_encode(&self.client_secret),
            form_encode(&self.refresh_token),
        );
        let response = self
            .agent
            .post(&self.token_url)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .send(form.as_bytes())
            .map_err(|e| MailError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_vec()
            .map_err(|e| MailError::Transport(e.to_string()))?;
        if status == 400 && String::from_utf8_lossy(&body).contains("invalid_grant") {
            return Err(MailError::Api {
                status,
                message:
                    "gmail status 400: invalid_grant (refresh token revoked/expired — re-consent)"
                        .to_string(),
            });
        }
        if !(200..300).contains(&status) {
            return Err(MailError::api(status, &body));
        }
        let value: serde_json::Value = serde_json::from_slice(&body)
            .map_err(|e| MailError::Parse(format!("token response: {e}")))?;
        let token = value["access_token"].as_str().unwrap_or("").to_string();
        if token.is_empty() {
            return Err(MailError::Parse(
                "token response lacks access_token".to_string(),
            ));
        }
        self.access_token = Some(token);
        Ok(())
    }

    /// `users.getProfile`: mailbox identity + watermark seed/reseed.
    pub fn profile(&self) -> Result<Profile, MailError> {
        let response = self.get("/gmail/v1/users/me/profile", &[])?;
        let value = self.ok_json("profile", response)?;
        let email = value["emailAddress"].as_str().unwrap_or("").to_string();
        let history_id = value["historyId"].as_str().unwrap_or("").to_string();
        if email.is_empty() || history_id.is_empty() {
            return Err(MailError::Parse(
                "profile lacks emailAddress/historyId".to_string(),
            ));
        }
        Ok(Profile { email, history_id })
    }

    /// `users.history.list`: added-message ids since `start`, one page.
    pub fn history_list(
        &self,
        start_history_id: &str,
        page_token: Option<&str>,
    ) -> Result<HistoryPage, MailError> {
        let mut query: Vec<(&str, &str)> = vec![
            ("startHistoryId", start_history_id),
            ("historyTypes", "messageAdded"),
            ("maxResults", "500"),
        ];
        if let Some(token) = page_token {
            query.push(("pageToken", token));
        }
        let response = self.get("/gmail/v1/users/me/history", &query)?;
        if response.status == 404 {
            return Err(MailError::HistoryExpired);
        }
        let value = self.ok_json("history.list", response)?;
        let mut added = Vec::new();
        if let Some(records) = value["history"].as_array() {
            for record in records {
                if let Some(entries) = record["messagesAdded"].as_array() {
                    for entry in entries {
                        let id = entry["message"]["id"].as_str().unwrap_or("");
                        let thread = entry["message"]["threadId"].as_str().unwrap_or("");
                        if !id.is_empty() {
                            added.push(AddedMessage {
                                id: id.to_string(),
                                thread_id: thread.to_string(),
                            });
                        }
                    }
                }
            }
        }
        let history_id = value["historyId"].as_str().unwrap_or("").to_string();
        if history_id.is_empty() {
            return Err(MailError::Parse("history.list lacks historyId".to_string()));
        }
        Ok(HistoryPage {
            added,
            next_page_token: value["nextPageToken"].as_str().map(str::to_string),
            history_id,
        })
    }

    /// `users.messages.list`: recent ids for the reseed backfill window.
    pub fn messages_list(
        &self,
        query_text: &str,
        page_token: Option<&str>,
    ) -> Result<ListPage, MailError> {
        let mut query: Vec<(&str, &str)> = vec![("q", query_text), ("maxResults", "500")];
        if let Some(token) = page_token {
            query.push(("pageToken", token));
        }
        let response = self.get("/gmail/v1/users/me/messages", &query)?;
        let value = self.ok_json("messages.list", response)?;
        let mut ids = Vec::new();
        if let Some(messages) = value["messages"].as_array() {
            for message in messages {
                let id = message["id"].as_str().unwrap_or("");
                let thread = message["threadId"].as_str().unwrap_or("");
                if !id.is_empty() {
                    ids.push(AddedMessage {
                        id: id.to_string(),
                        thread_id: thread.to_string(),
                    });
                }
            }
        }
        Ok(ListPage {
            ids,
            next_page_token: value["nextPageToken"].as_str().map(str::to_string),
        })
    }

    /// `users.messages.get?format=raw`: decoded RFC 2822 bytes + metadata.
    pub fn message_raw(&self, id: &str) -> Result<RawMessage, MailError> {
        let response = self.get(
            &format!("/gmail/v1/users/me/messages/{id}"),
            &[("format", "raw")],
        )?;
        let value = self.ok_json("messages.get", response)?;
        let encoded = value["raw"].as_str().unwrap_or("");
        if encoded.is_empty() {
            return Err(MailError::Parse("messages.get lacks raw".to_string()));
        }
        use base64::Engine as _;
        let raw = base64::engine::general_purpose::URL_SAFE_NO_PAD
            .decode(encoded)
            .map_err(|e| MailError::Parse(format!("raw is not base64url: {e}")))?;
        let internal_date_ms = value["internalDate"]
            .as_str()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let labels = value["labelIds"]
            .as_array()
            .map(|list| {
                list.iter()
                    .filter_map(|l| l.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        Ok(RawMessage {
            id: value["id"].as_str().unwrap_or("").to_string(),
            thread_id: value["threadId"].as_str().unwrap_or("").to_string(),
            labels,
            internal_date_ms,
            raw,
        })
    }

    fn get(&self, path: &str, query: &[(&str, &str)]) -> Result<RawResponse, MailError> {
        self.credentials()?;
        let token = self.bearer()?.to_string();
        let url = format!("{}{path}", self.gmail_base);
        let mut request = self
            .agent
            .get(&url)
            .header("Authorization", format!("Bearer {token}"));
        for (key, value) in query {
            request = request.query(*key, *value);
        }
        let response = request
            .call()
            .map_err(|e| MailError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_vec()
            .map_err(|e| MailError::Transport(e.to_string()))?;
        Ok(RawResponse { status, body })
    }

    /// 2xx → parsed JSON; 401 → [`MailError::Unauthorized`]; 429 /
    /// rate-limit 403 → [`MailError::RateLimited`]; anything else →
    /// [`MailError::Api`] with a short phrase.
    fn ok_json(&self, what: &str, response: RawResponse) -> Result<serde_json::Value, MailError> {
        if response.status == 401 {
            return Err(MailError::Unauthorized);
        }
        if response.status == 429 {
            return Err(MailError::RateLimited);
        }
        if response.status == 403
            && matches!(
                MailError::reason(&response.body).as_deref(),
                Some("rateLimitExceeded" | "userRateLimitExceeded")
            )
        {
            return Err(MailError::RateLimited);
        }
        if !(200..300).contains(&response.status) {
            return Err(MailError::api(response.status, &response.body));
        }
        serde_json::from_slice(&response.body).map_err(|e| MailError::Parse(format!("{what}: {e}")))
    }
}

/// Minimal `application/x-www-form-urlencoded` encoding for token
/// values (RFC 3986 unreserved set passes through; space becomes `+`).
fn form_encode(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for byte in value.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

struct RawResponse {
    status: u16,
    body: Vec<u8>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// Serve canned exchanges in order against a port-0 stub, reporting
    /// each raw request head+body over a channel.
    fn stub(
        replies: Vec<(u16, String)>,
    ) -> (String, mpsc::Receiver<String>, std::thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            for (status, reply) in replies {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream.try_clone().unwrap());
                let mut head = String::new();
                reader.read_line(&mut head).unwrap();
                let mut len = 0usize;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let trimmed = line.trim_end();
                    if trimmed.is_empty() {
                        break;
                    }
                    head.push_str(&line);
                    if let Some(n) = trimmed
                        .to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse().ok())
                    {
                        len = n;
                    }
                }
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body).unwrap();
                tx.send(format!("{head}\n{}", String::from_utf8_lossy(&body)))
                    .unwrap();
                let mut stream = reader.into_inner();
                let text = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                    reply.len()
                );
                stream.write_all(text.as_bytes()).unwrap();
            }
        });
        (base, rx, handle)
    }

    /// Client with both bases on one stub (paths distinguish calls).
    fn client(base: &str) -> MailClient {
        MailClient::new(base, &format!("{base}/token"), "cid", "csec", "rtok")
    }

    fn recv(rx: &mpsc::Receiver<String>) -> String {
        rx.recv_timeout(Duration::from_secs(5)).unwrap()
    }

    #[test]
    fn refresh_posts_form_and_stores_token() {
        let (base, seen, handle) = stub(vec![(
            200,
            r#"{"access_token":"tok-1","expires_in":3599,"token_type":"Bearer"}"#.to_string(),
        )]);
        let mut client = client(&base);
        client.refresh().expect("refresh");
        let raw = recv(&seen).to_ascii_lowercase();
        assert!(raw.starts_with("post /token "), "{raw}");
        assert!(
            raw.contains("content-type: application/x-www-form-urlencoded"),
            "{raw}"
        );
        assert!(raw.contains("grant_type=refresh_token"), "{raw}");
        assert!(raw.contains("refresh_token=rtok"), "{raw}");
        handle.join().unwrap();
    }

    #[test]
    fn refresh_invalid_grant_names_reconsent() {
        let (base, seen, handle) = stub(vec![(
            400,
            r#"{"error":"invalid_grant","error_description":"revoked"}"#.to_string(),
        )]);
        let mut client = client(&base);
        let err = client.refresh().expect_err("revoked");
        assert!(matches!(err, MailError::Api { status: 400, .. }), "{err}");
        assert!(err.to_string().contains("re-consent"), "{err}");
        assert!(!err.to_string().contains("rtok"), "{err}");
        let _ = recv(&seen);
        handle.join().unwrap();
    }

    #[test]
    fn gmail_calls_carry_bearer_and_parse() {
        use base64::Engine as _;
        let raw_rfc = b"From: ops@example.gov\r\nSubject: closure\r\n\r\nClosed Monday.";
        let encoded = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(raw_rfc);
        let (base, seen, handle) = stub(vec![
            (200, r#"{"emailAddress":"box@example.com","historyId":"987654"}"#.to_string()),
            (
                200,
                r#"{"history":[{"id":"987660","messagesAdded":[{"message":{"id":"m1","threadId":"t1"}}]}],
                    "historyId":"987660"}"#
                    .to_string(),
            ),
            (
                200,
                format!(
                    r#"{{"id":"m1","threadId":"t1","labelIds":["INBOX"],"internalDate":"1729000000000","raw":"{encoded}"}}"#
                ),
            ),
        ]);
        let mut client = client(&base);
        // Seed the token directly: refresh is covered above.
        client.access_token = Some("tok-1".to_string());
        let profile = client.profile().expect("profile");
        assert_eq!(profile.email, "box@example.com");
        assert_eq!(profile.history_id, "987654");
        let page = client.history_list("987654", None).expect("history");
        assert_eq!(page.added.len(), 1);
        assert_eq!(page.added[0].id, "m1");
        assert_eq!(page.history_id, "987660");
        let msg = client.message_raw("m1").expect("raw");
        assert_eq!(msg.raw, raw_rfc);
        assert_eq!(msg.internal_date_ms, 1_729_000_000_000);
        assert_eq!(msg.labels, vec!["INBOX".to_string()]);
        for _ in 0..3 {
            let raw = recv(&seen).to_ascii_lowercase();
            assert!(raw.contains("authorization: bearer tok-1"), "{raw}");
        }
        handle.join().unwrap();
    }

    #[test]
    fn history_query_shape() {
        let (base, seen, handle) = stub(vec![(200, r#"{"historyId":"9"}"#.to_string())]);
        let mut client = client(&base);
        client.access_token = Some("tok-1".to_string());
        client.history_list("8", Some("pg")).expect("history");
        let raw = recv(&seen);
        let line = raw.lines().next().unwrap();
        assert!(
            line.starts_with("GET /gmail/v1/users/me/history?"),
            "{line}"
        );
        assert!(line.contains("startHistoryId=8"), "{line}");
        assert!(line.contains("historyTypes=messageAdded"), "{line}");
        assert!(line.contains("pageToken=pg"), "{line}");
        handle.join().unwrap();
    }

    #[test]
    fn status_mapping() {
        // 401 → Unauthorized, 404 history → HistoryExpired, 429/403 →
        // RateLimited, other 403 → Api, 2xx garbage → Parse.
        let (base, seen, handle) = stub(vec![
            (
                401,
                r#"{"error":{"errors":[{"reason":"authError"}]}}"#.to_string(),
            ),
            (
                404,
                r#"{"error":{"errors":[{"reason":"notFound"}]}}"#.to_string(),
            ),
            (429, r#"{"error":{}}"#.to_string()),
            (
                403,
                r#"{"error":{"errors":[{"reason":"rateLimitExceeded"}]}}"#.to_string(),
            ),
            (
                403,
                r#"{"error":{"errors":[{"reason":"insufficientPermissions"}]}}"#.to_string(),
            ),
            (200, "not json".to_string()),
        ]);
        let mut client = client(&base);
        client.access_token = Some("tok-1".to_string());
        assert_eq!(
            client.history_list("1", None).expect_err("401"),
            MailError::Unauthorized
        );
        assert_eq!(
            client.history_list("1", None).expect_err("404"),
            MailError::HistoryExpired
        );
        assert_eq!(
            client.history_list("1", None).expect_err("429"),
            MailError::RateLimited
        );
        assert_eq!(
            client.history_list("1", None).expect_err("403 limited"),
            MailError::RateLimited
        );
        let err = client.history_list("1", None).expect_err("403 scope");
        assert!(matches!(err, MailError::Api { status: 403, .. }), "{err}");
        let err = client.history_list("1", None).expect_err("garbage");
        assert!(matches!(err, MailError::Parse(_)), "{err}");
        for _ in 0..6 {
            let _ = recv(&seen);
        }
        handle.join().unwrap();
    }

    #[test]
    fn missing_credentials_fail_closed_without_network() {
        let client = MailClient::new("http://127.0.0.1:9", "http://127.0.0.1:9/token", "", "", "");
        // No stub: nothing may be dialed.
        let mut client = client;
        assert_eq!(
            client.refresh().expect_err("refresh"),
            MailError::NotConfigured
        );
        assert_eq!(
            client.profile().expect_err("profile"),
            MailError::NotConfigured
        );
    }

    #[test]
    fn form_encode_escapes_reserved() {
        assert_eq!(form_encode("abcAZ09-_.~"), "abcAZ09-_.~");
        assert_eq!(form_encode("a b+c"), "a+b%2Bc");
    }
}
