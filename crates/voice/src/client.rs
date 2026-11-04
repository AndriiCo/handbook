//! Blocking Telnyx Call Control client (runs on `spawn_blocking`).
//!
//! Base-URL override exists so tests dial a stub server; production
//! points at `https://api.telnyx.com`. The API key travels only in
//! the `Authorization` header and never appears in errors or logs.

use std::fmt;
use std::time::Duration;

use serde_json::json;

/// Default Telnyx API root; tests override with a stub URL.
pub const TELNYX_BASE_URL: &str = "https://api.telnyx.com";

/// Voice configuration from sealed DB rows (`telnyx.*` keys). The
/// API key travels separately (never in this struct, so a `Debug`
/// dump can never leak it). `Default` is all-empty, which reads as
/// unconfigured (fail closed).
#[derive(Debug, Clone, Default)]
pub struct VoiceConfig {
    /// API root override (`telnyx.base_url`, default Telnyx prod).
    pub base_url: String,
    /// Caller number on the account, E.164 (`telnyx.from_number`).
    pub from_number: String,
    /// Voice API application id (`telnyx.connection_id`).
    pub connection_id: String,
    /// Webhook signing public key, base64 (`telnyx.webhook_public_key`).
    pub webhook_public_key: String,
    /// Credential connection the staff browser phones register on
    /// (`telnyx.webrtc_connection_id`); empty = no browser audio, so
    /// real dials refuse.
    pub webrtc_connection_id: String,
    /// Script-pack directory override; empty reads as the default
    /// (`crates/voice/scripts`, repo-relative). Tests point this at
    /// a temp dir of fixture packs.
    pub scripts_dir: String,
}

impl VoiceConfig {
    /// Build from stored values (missing → empty, base URL → prod).
    pub fn from_values(
        base_url: &str,
        from_number: &str,
        connection_id: &str,
        webhook_public_key: &str,
    ) -> Self {
        VoiceConfig {
            base_url: if base_url.trim().is_empty() {
                TELNYX_BASE_URL.to_string()
            } else {
                base_url.trim_end_matches('/').to_string()
            },
            from_number: from_number.to_string(),
            connection_id: connection_id.to_string(),
            webhook_public_key: webhook_public_key.to_string(),
            webrtc_connection_id: String::new(),
            scripts_dir: crate::scripts::DEFAULT_SCRIPTS_DIR.to_string(),
        }
    }

    /// Set the browser-phone credential connection
    /// (`telnyx.webrtc_connection_id`).
    #[must_use]
    pub fn with_webrtc_connection(mut self, connection_id: &str) -> Self {
        self.webrtc_connection_id = connection_id.trim().to_string();
        self
    }
}

/// SIP domain a telephony credential registers on: a browser logged
/// in with a credential's JWT rings when `sip:<sip_username>@` this
/// domain is dialed (Telnyx WebRTC fundamentals, "Dialing Registered
/// Clients"; the credential connection needs
/// `sip_uri_calling_preference = internal`).
pub const TELNYX_SIP_DOMAIN: &str = "sip.telnyx.com";

/// A staff browser phone's telephony credential. Only the id and SIP
/// username are kept: the SIP password and token material in the
/// create response are dropped unread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PhoneCredential {
    pub id: String,
    pub sip_username: String,
}

/// Blocking Telnyx client. Cheap to clone; safe to move across the
/// `spawn_blocking` boundary the web layer uses.
#[derive(Debug, Clone)]
pub struct TelnyxClient {
    agent: ureq::Agent,
    base_url: String,
    api_key: String,
    connection_id: String,
    from_number: String,
}

impl TelnyxClient {
    /// Build a client; an empty key fails closed at call time.
    pub fn new(base_url: &str, key: &str, connection_id: &str, from_number: &str) -> Self {
        let config = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_global(Some(Duration::from_secs(30)))
            .build();
        TelnyxClient {
            agent: ureq::Agent::new_with_config(config),
            base_url: base_url.trim_end_matches('/').to_string(),
            api_key: key.to_string(),
            connection_id: connection_id.to_string(),
            from_number: from_number.to_string(),
        }
    }

    /// Dial `to` (E.164) from the account number. Returns the Telnyx
    /// leg/session ids for the call row.
    ///
    /// # Errors
    ///
    /// [`VoiceError::NotConfigured`] when key/connection/from is
    /// missing; [`VoiceError::Transport`] on I/O; [`VoiceError::Api`]
    /// when Telnyx answers non-2xx.
    pub fn dial(&self, to: &str, command_id: &str) -> Result<DialOutcome, VoiceError> {
        self.dial_with_state(to, command_id, None)
    }

    /// Ring a staff browser phone (`sip:<sip_username>@sip.telnyx.com`)
    /// from the Call Control app. `client_state` (base64) comes back
    /// on every webhook for this leg.
    ///
    /// # Errors
    ///
    /// Same as [`TelnyxClient::dial`].
    pub fn dial_browser(
        &self,
        sip_username: &str,
        command_id: &str,
        client_state: &str,
    ) -> Result<DialOutcome, VoiceError> {
        let to = format!("sip:{sip_username}@{TELNYX_SIP_DOMAIN}");
        self.dial_with_state(&to, command_id, Some(client_state))
    }

    fn dial_with_state(
        &self,
        to: &str,
        command_id: &str,
        client_state: Option<&str>,
    ) -> Result<DialOutcome, VoiceError> {
        if self.api_key.is_empty() || self.connection_id.is_empty() || self.from_number.is_empty() {
            return Err(VoiceError::NotConfigured);
        }
        let mut body = json!({
            "to": to,
            "from": self.from_number,
            "connection_id": self.connection_id,
            "command_id": command_id,
            "webhook_url_method": "POST",
        });
        if let Some(state) = client_state {
            body["client_state"] = json!(state);
        }
        let response = self.post_json(&format!("{}/v2/calls", self.base_url), &body)?;
        if !(200..300).contains(&response.status) {
            return Err(VoiceError::api(response.status, &response.body));
        }
        let value: serde_json::Value = serde_json::from_slice(&response.body)
            .map_err(|e| VoiceError::Parse(format!("dial response: {e}")))?;
        let data = &value["data"];
        let leg = data["call_control_id"].as_str().unwrap_or("").to_string();
        if leg.is_empty() {
            return Err(VoiceError::Parse(
                "dial response lacks call_control_id".to_string(),
            ));
        }
        Ok(DialOutcome {
            call_control_id: leg,
            call_leg_id: data["call_leg_id"].as_str().unwrap_or("").to_string(),
            call_session_id: data["call_session_id"].as_str().unwrap_or("").to_string(),
        })
    }

    /// Start recording (`mp3`, dual channel) on a live leg.
    ///
    /// # Errors
    ///
    /// Same as [`TelnyxClient::dial`].
    pub fn record_start(&self, call_control_id: &str, command_id: &str) -> Result<(), VoiceError> {
        self.action(
            call_control_id,
            "record_start",
            &json!({"format": "mp3", "channels": "dual", "command_id": command_id}),
        )
    }

    /// Stop recording on a live leg.
    ///
    /// # Errors
    ///
    /// Same as [`TelnyxClient::dial`].
    pub fn record_stop(&self, call_control_id: &str, command_id: &str) -> Result<(), VoiceError> {
        self.action(
            call_control_id,
            "record_stop",
            &json!({"command_id": command_id}),
        )
    }

    /// Start live transcription; segments arrive as webhooks.
    ///
    /// # Errors
    ///
    /// Same as [`TelnyxClient::dial`].
    pub fn transcription_start(
        &self,
        call_control_id: &str,
        command_id: &str,
    ) -> Result<(), VoiceError> {
        self.action(
            call_control_id,
            "transcription_start",
            &json!({"command_id": command_id}),
        )
    }

    /// Stop live transcription on a live leg.
    ///
    /// # Errors
    ///
    /// Same as [`TelnyxClient::dial`].
    pub fn transcription_stop(
        &self,
        call_control_id: &str,
        command_id: &str,
    ) -> Result<(), VoiceError> {
        self.action(
            call_control_id,
            "transcription_stop",
            &json!({"command_id": command_id}),
        )
    }

    /// Join `other_leg` to `call_control_id`: both parties hear each
    /// other (`POST /v2/calls/{id}/actions/bridge`).
    ///
    /// # Errors
    ///
    /// Same as [`TelnyxClient::dial`].
    pub fn bridge(
        &self,
        call_control_id: &str,
        other_leg: &str,
        command_id: &str,
    ) -> Result<(), VoiceError> {
        self.action(
            call_control_id,
            "bridge",
            &json!({"call_control_id": other_leg, "command_id": command_id}),
        )
    }

    /// Create a telephony credential on the browser-phone credential
    /// connection (`POST /v2/telephony_credentials`).
    ///
    /// # Errors
    ///
    /// [`VoiceError::NotConfigured`] without a key or connection;
    /// otherwise as [`TelnyxClient::dial`].
    pub fn create_phone_credential(
        &self,
        credential_connection_id: &str,
        name: &str,
    ) -> Result<PhoneCredential, VoiceError> {
        if self.api_key.is_empty() || credential_connection_id.is_empty() {
            return Err(VoiceError::NotConfigured);
        }
        let body = json!({
            "connection_id": credential_connection_id,
            "name": name,
            "tag": "apostille-staff",
        });
        let response = self.post_json(
            &format!("{}/v2/telephony_credentials", self.base_url),
            &body,
        )?;
        if !(200..300).contains(&response.status) {
            return Err(VoiceError::api(response.status, &response.body));
        }
        // The body also holds the SIP password and token material:
        // read two fields, never echo the body.
        let value: serde_json::Value = serde_json::from_slice(&response.body)
            .map_err(|_| VoiceError::Parse("credential response is not JSON".to_string()))?;
        let field = |name: &str| value["data"][name].as_str().unwrap_or("").to_string();
        let (id, sip_username) = (field("id"), field("sip_username"));
        if id.is_empty() || sip_username.is_empty() {
            return Err(VoiceError::Parse(
                "credential response lacks id or sip_username".to_string(),
            ));
        }
        Ok(PhoneCredential { id, sip_username })
    }

    /// Mint a browser login JWT for a telephony credential (`POST
    /// /v2/telephony_credentials/{id}/token`, plain-text body, valid
    /// 24 h). The token logs a browser in as that credential: hand it
    /// to its staff member only, never log it.
    ///
    /// # Errors
    ///
    /// Same as [`TelnyxClient::dial`]; a deleted credential is
    /// `Api { status: 404, .. }`.
    pub fn phone_token(&self, credential_id: &str) -> Result<String, VoiceError> {
        if self.api_key.is_empty() {
            return Err(VoiceError::NotConfigured);
        }
        let url = format!(
            "{}/v2/telephony_credentials/{credential_id}/token",
            self.base_url
        );
        let response = self
            .agent
            .post(&url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .send_empty()
            .map_err(|e| VoiceError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_vec()
            .map_err(|e| VoiceError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(VoiceError::api(status, &body));
        }
        let token = String::from_utf8_lossy(&body).trim().to_string();
        let jwt_shaped = token.split('.').count() == 3
            && token
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.' | b'-' | b'_'));
        if !jwt_shaped {
            return Err(VoiceError::Parse("token response is not a JWT".to_string()));
        }
        Ok(token)
    }

    /// Hang up a leg (answered or still ringing).
    ///
    /// # Errors
    ///
    /// Same as [`TelnyxClient::dial`].
    pub fn hangup(&self, call_control_id: &str, command_id: &str) -> Result<(), VoiceError> {
        self.action(
            call_control_id,
            "hangup",
            &json!({"command_id": command_id}),
        )
    }

    /// Download bytes from an absolute URL with the API key (fetches
    /// `recording_urls` entries after `call.recording.saved`). Only
    /// `https` URLs are fetched; anything else is refused without a
    /// request.
    ///
    /// # Errors
    ///
    /// [`VoiceError::Transport`] on I/O or non-2xx; [`VoiceError::Api`]
    /// when the URL is not `https`.
    pub fn download(&self, url: &str) -> Result<Vec<u8>, VoiceError> {
        if self.api_key.is_empty() {
            return Err(VoiceError::NotConfigured);
        }
        if !url.starts_with("https://") {
            return Err(VoiceError::Api {
                status: 0,
                message: "refusing non-https download".to_string(),
            });
        }
        let response = self
            .agent
            .get(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .call()
            .map_err(|e| VoiceError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let bytes = response
            .into_body()
            .read_to_vec()
            .map_err(|e| VoiceError::Transport(e.to_string()))?;
        if !(200..300).contains(&status) {
            return Err(VoiceError::Api {
                status,
                message: format!("download failed with status {status}"),
            });
        }
        Ok(bytes)
    }

    fn action(
        &self,
        call_control_id: &str,
        action: &str,
        body: &serde_json::Value,
    ) -> Result<(), VoiceError> {
        if self.api_key.is_empty() {
            return Err(VoiceError::NotConfigured);
        }
        let url = format!(
            "{}/v2/calls/{call_control_id}/actions/{action}",
            self.base_url
        );
        let response = self.post_json(&url, body)?;
        if (200..300).contains(&response.status) {
            Ok(())
        } else {
            Err(VoiceError::api(response.status, &response.body))
        }
    }

    fn post_json(&self, url: &str, body: &serde_json::Value) -> Result<RawResponse, VoiceError> {
        let response = self
            .agent
            .post(url)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .send_json(body)
            .map_err(|e| VoiceError::Transport(e.to_string()))?;
        let status = response.status().as_u16();
        let body = response
            .into_body()
            .read_to_vec()
            .map_err(|e| VoiceError::Transport(e.to_string()))?;
        Ok(RawResponse { status, body })
    }
}

struct RawResponse {
    status: u16,
    body: Vec<u8>,
}

/// Leg ids returned by [`TelnyxClient::dial`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialOutcome {
    pub call_control_id: String,
    pub call_leg_id: String,
    pub call_session_id: String,
}

/// Telnyx client failure. Messages carry status codes and Telnyx's
/// own error entries (code, title, detail, field) — never the API
/// key, and phone-number-shaped digits are masked (error details
/// can echo the request, which holds the callee number).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VoiceError {
    /// Key, connection, or from-number missing: fail closed.
    NotConfigured,
    /// I/O, TLS, timeout, or protocol failure.
    Transport(String),
    /// Telnyx answered non-2xx (or a download URL was refused).
    Api { status: u16, message: String },
    /// A 2xx response did not parse as expected.
    Parse(String),
}

/// Backstop cap on an API error message, in chars.
const MAX_ERROR_CHARS: usize = 1000;

impl VoiceError {
    /// Every Telnyx `errors[]` entry on one line, so a journal grep
    /// shows the whole reason; a non-JSON body is kept with its
    /// whitespace collapsed. Phone numbers are masked either way.
    fn api(status: u16, body: &[u8]) -> Self {
        let text = String::from_utf8_lossy(body);
        let summary = telnyx_errors(&text)
            .unwrap_or_else(|| text.split_whitespace().collect::<Vec<_>>().join(" "));
        let summary: String = mask_numbers(&summary)
            .chars()
            .take(MAX_ERROR_CHARS)
            .collect();
        VoiceError::Api {
            status,
            message: format!("telnyx status {status}: {summary}"),
        }
    }
}

/// `code title: detail (at /pointer)` per entry, joined by `; `.
/// `None` when the body is not a Telnyx error envelope.
fn telnyx_errors(text: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(text).ok()?;
    let errors = value["errors"].as_array().filter(|e| !e.is_empty())?;
    let field = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => s.split_whitespace().collect::<Vec<_>>().join(" "),
        serde_json::Value::Null => String::new(),
        other => other.to_string(),
    };
    let entries: Vec<String> = errors
        .iter()
        .map(|e| {
            let code = field(&e["code"]);
            let title = field(&e["title"]);
            let detail = field(&e["detail"]);
            let mut out = [code.as_str(), title.as_str()]
                .into_iter()
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            if !detail.is_empty() && detail != title {
                if !out.is_empty() {
                    out.push_str(": ");
                }
                out.push_str(&detail);
            }
            let at = field(&e["source"]["pointer"]);
            let at = if at.is_empty() {
                field(&e["source"]["parameter"])
            } else {
                at
            };
            if !at.is_empty() {
                out.push_str(&format!(" (at {at})"));
            }
            out
        })
        .collect();
    Some(entries.join("; "))
}

/// Mask phone-number-shaped digit runs: any `+` followed by digits,
/// or a bare run of 10–15 digits (E.164 without the plus). Longer
/// runs, such as 19-digit Telnyx app ids, stay readable.
fn mask_numbers(text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < chars.len() {
        let plus = chars[i] == '+' && chars.get(i + 1).is_some_and(char::is_ascii_digit);
        if plus || chars[i].is_ascii_digit() {
            let start = i;
            i += usize::from(plus);
            while i < chars.len() && chars[i].is_ascii_digit() {
                i += 1;
            }
            let digits = i - start - usize::from(plus);
            if plus || (10..=15).contains(&digits) {
                out.push_str("[number]");
            } else {
                out.extend(&chars[start..i]);
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

impl fmt::Display for VoiceError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            VoiceError::NotConfigured => f.write_str("voice not configured"),
            VoiceError::Transport(message) => write!(f, "voice transport: {message}"),
            VoiceError::Api { message, .. } => write!(f, "voice api: {message}"),
            VoiceError::Parse(message) => write!(f, "voice parse: {message}"),
        }
    }
}

impl std::error::Error for VoiceError {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader, Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// One canned HTTP/1.1 exchange against a port-0 stub: the test
    /// hands the stub a responder closure, the stub binds
    /// 127.0.0.1:0 and reports its base URL.
    fn stub<F>(respond: F) -> (String, mpsc::Receiver<String>, std::thread::JoinHandle<()>)
    where
        F: FnOnce(&str, &str) -> (u16, String) + Send + 'static,
    {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://127.0.0.1:{}", listener.local_addr().unwrap().port());
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            let mut head = String::new();
            reader.read_line(&mut head).unwrap();
            let mut len = 0usize;
            let mut chunked = false;
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                let trimmed = line.trim_end();
                if trimmed.is_empty() {
                    break;
                }
                head.push_str(&line);
                let lower = trimmed.to_ascii_lowercase();
                if let Some(n) = lower
                    .strip_prefix("content-length:")
                    .and_then(|v| v.trim().parse().ok())
                {
                    len = n;
                }
                if lower == "transfer-encoding: chunked" {
                    chunked = true;
                }
            }
            let body = if chunked {
                let mut raw = Vec::new();
                loop {
                    let mut size_line = String::new();
                    reader.read_line(&mut size_line).unwrap();
                    let size = usize::from_str_radix(size_line.trim(), 16).unwrap();
                    if size == 0 {
                        reader.read_line(&mut String::new()).unwrap();
                        break;
                    }
                    let mut chunk = vec![0u8; size];
                    reader.read_exact(&mut chunk).unwrap();
                    raw.extend_from_slice(&chunk);
                    reader.read_line(&mut String::new()).unwrap();
                }
                String::from_utf8_lossy(&raw).to_string()
            } else {
                let mut body = vec![0u8; len];
                reader.read_exact(&mut body).unwrap();
                String::from_utf8_lossy(&body).to_string()
            };
            let (status, reply) = respond(&head, &body);
            tx.send(format!("{head}\n{body}")).unwrap();
            let mut stream = reader.into_inner();
            let text = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{reply}",
                reply.len()
            );
            stream.write_all(text.as_bytes()).unwrap();
        });
        (base, rx, handle)
    }

    fn client(base: &str) -> TelnyxClient {
        TelnyxClient::new(base, "stub-key", "conn-1", "+13605550000")
    }

    #[test]
    fn dial_posts_calls_and_reads_ids() {
        let (base, seen, handle) = stub(|_head, body| {
            let v: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["to"], "+13605550100");
            assert_eq!(v["from"], "+13605550000");
            assert_eq!(v["connection_id"], "conn-1");
            assert!(v["command_id"].as_str().is_some_and(|s| !s.is_empty()));
            (
                200,
                r#"{"data":{"call_control_id":"leg-1","call_leg_id":"leg-1","call_session_id":"sess-1"}}"#.to_string(),
            )
        });
        let out = client(&base).dial("+13605550100", "cmd-1").unwrap();
        assert_eq!(out.call_control_id, "leg-1");
        assert_eq!(out.call_session_id, "sess-1");
        let raw = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(raw.starts_with("POST /v2/calls "), "{raw}");
        assert!(
            raw.to_ascii_lowercase()
                .contains("authorization: bearer stub-key"),
            "{raw}"
        );
        handle.join().unwrap();
    }

    #[test]
    fn actions_hit_the_leg_action_path() {
        for action in [
            "record_start",
            "record_stop",
            "transcription_start",
            "hangup",
        ] {
            let (base, seen, handle) = stub(|_, _| (200, r#"{"data":{}}"#.to_string()));
            let c = client(&base);
            let ok = match action {
                "record_start" => c.record_start("leg-1", "cmd-1"),
                "record_stop" => c.record_stop("leg-1", "cmd-1"),
                "transcription_start" => c.transcription_start("leg-1", "cmd-1"),
                _ => c.hangup("leg-1", "cmd-1"),
            };
            ok.unwrap();
            let raw = seen.recv_timeout(Duration::from_secs(5)).unwrap();
            assert!(
                raw.starts_with(&format!("POST /v2/calls/leg-1/actions/{action} ")),
                "{raw}"
            );
            handle.join().unwrap();
        }
    }

    #[test]
    fn dial_browser_rings_the_credential_sip_uri_with_state() {
        let (base, _seen, handle) = stub(|_head, body| {
            let v: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["to"], "sip:gencredabc@sip.telnyx.com");
            assert_eq!(v["from"], "+13605550000");
            assert_eq!(v["connection_id"], "conn-1");
            assert_eq!(v["client_state"], "c3RhdGU=");
            (
                200,
                r#"{"data":{"call_control_id":"web-1","call_leg_id":"web-1","call_session_id":"s"}}"#
                    .to_string(),
            )
        });
        let out = client(&base)
            .dial_browser("gencredabc", "cmd-1", "c3RhdGU=")
            .unwrap();
        assert_eq!(out.call_control_id, "web-1");
        handle.join().unwrap();
    }

    #[test]
    fn bridge_joins_the_other_leg() {
        let (base, seen, handle) = stub(|_, body| {
            let v: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["call_control_id"], "office-1");
            (200, r#"{"data":{"result":"ok"}}"#.to_string())
        });
        client(&base).bridge("web-1", "office-1", "cmd-1").unwrap();
        let raw = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            raw.starts_with("POST /v2/calls/web-1/actions/bridge "),
            "{raw}"
        );
        handle.join().unwrap();
    }

    #[test]
    fn phone_credential_keeps_id_and_sip_username_only() {
        let (base, seen, handle) = stub(|_, body| {
            let v: serde_json::Value = serde_json::from_str(body).unwrap();
            assert_eq!(v["connection_id"], "cred-conn");
            assert_eq!(v["name"], "apostille-staff-1");
            (
                200,
                r#"{"data":{"id":"tc-1","sip_username":"gencredabc","sip_password":"pw-never-kept","token_material":{"signing_secret":"x"}}}"#
                    .to_string(),
            )
        });
        let cred = client(&base)
            .create_phone_credential("cred-conn", "apostille-staff-1")
            .unwrap();
        assert_eq!(
            cred,
            PhoneCredential {
                id: "tc-1".to_string(),
                sip_username: "gencredabc".to_string(),
            }
        );
        assert!(!format!("{cred:?}").contains("pw-never-kept"));
        let raw = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(raw.starts_with("POST /v2/telephony_credentials "), "{raw}");
        handle.join().unwrap();

        let (base, _seen, handle) = stub(|_, _| {
            (
                200,
                r#"{"data":{"sip_password":"pw-never-kept"}}"#.to_string(),
            )
        });
        let err = client(&base)
            .create_phone_credential("cred-conn", "n")
            .unwrap_err();
        assert!(!err.to_string().contains("pw-never-kept"), "{err}");
        handle.join().unwrap();

        let c = client("http://127.0.0.1:9");
        assert_eq!(
            c.create_phone_credential("", "n").unwrap_err(),
            VoiceError::NotConfigured
        );
    }

    #[test]
    fn phone_token_reads_the_plain_text_jwt() {
        let (base, seen, handle) = stub(|_, _| (200, "eyJhbGciOi.eyJzdWIiOi.c2ln\n".to_string()));
        assert_eq!(
            client(&base).phone_token("tc-1").unwrap(),
            "eyJhbGciOi.eyJzdWIiOi.c2ln"
        );
        let raw = seen.recv_timeout(Duration::from_secs(5)).unwrap();
        assert!(
            raw.starts_with("POST /v2/telephony_credentials/tc-1/token "),
            "{raw}"
        );
        handle.join().unwrap();

        let (base, _seen, handle) = stub(|_, _| {
            (
                404,
                r#"{"errors":[{"code":"10005","title":"Resource not found"}]}"#.to_string(),
            )
        });
        let err = client(&base).phone_token("gone").unwrap_err();
        assert!(matches!(err, VoiceError::Api { status: 404, .. }), "{err}");
        handle.join().unwrap();

        let (base, _seen, handle) = stub(|_, _| (200, "<html>nope</html>".to_string()));
        assert!(matches!(
            client(&base).phone_token("tc-1").unwrap_err(),
            VoiceError::Parse(_)
        ));
        handle.join().unwrap();
    }

    #[test]
    fn api_errors_carry_status_not_numbers() {
        let (base, _seen, handle) = stub(|_, _| {
            (
                422,
                r#"{"errors":[{"detail":"bad number +13605550100 (13605550100)"}]}"#.to_string(),
            )
        });
        let err = client(&base).dial("+13605550100", "cmd-1").unwrap_err();
        assert!(matches!(err, VoiceError::Api { status: 422, .. }), "{err}");
        // The callee number must not leak into the error text.
        assert!(!err.to_string().contains("3605550100"), "{err}");
        assert!(
            err.to_string().ends_with("bad number [number] ([number])"),
            "{err}"
        );
        handle.join().unwrap();
    }

    #[test]
    fn api_errors_keep_every_telnyx_entry_on_one_line() {
        let body = r#"{
  "errors": [
    {
      "code": "10015",
      "title": "Invalid value for connection_id (Call Control App ID)",
      "detail": "The connection_id 3034092385005995241 is not a Call Control App on this account.",
      "source": {"pointer": "/connection_id"}
    },
    {"code": 10016, "title": "Bad from", "detail": "Bad from", "source": {"parameter": "from"}}
  ]
}"#;
        let err = VoiceError::api(422, body.as_bytes());
        assert_eq!(
            err.to_string(),
            "voice api: telnyx status 422: 10015 Invalid value for connection_id \
             (Call Control App ID): The connection_id 3034092385005995241 is not a \
             Call Control App on this account. (at /connection_id); \
             10016 Bad from (at from)"
        );
    }

    #[test]
    fn non_json_api_errors_are_kept_whole_on_one_line() {
        let err = VoiceError::api(502, b"<html>\n  <body>Bad   gateway</body>\n</html>");
        assert_eq!(
            err.to_string(),
            "voice api: telnyx status 502: <html> <body>Bad gateway</body> </html>"
        );
        let long = "x".repeat(MAX_ERROR_CHARS * 2);
        let VoiceError::Api { message, .. } = VoiceError::api(500, long.as_bytes()) else {
            panic!("api() must build VoiceError::Api");
        };
        assert_eq!(
            message.chars().count(),
            "telnyx status 500: ".len() + MAX_ERROR_CHARS
        );
    }

    #[test]
    fn missing_config_fails_closed() {
        let c = TelnyxClient::new("http://127.0.0.1:9", "", "", "");
        assert_eq!(
            c.dial("+13605550100", "cmd-1").unwrap_err(),
            VoiceError::NotConfigured
        );
        assert_eq!(
            c.hangup("leg-1", "cmd-1").unwrap_err(),
            VoiceError::NotConfigured
        );
        assert_eq!(
            c.download("https://example.test/r.mp3").unwrap_err(),
            VoiceError::NotConfigured
        );
    }

    #[test]
    fn download_refuses_plain_http() {
        let c = client("http://127.0.0.1:9");
        let err = c.download("http://example.test/r.mp3").unwrap_err();
        assert!(matches!(err, VoiceError::Api { status: 0, .. }), "{err}");
    }
}
