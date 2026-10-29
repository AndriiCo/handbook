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
            scripts_dir: crate::scripts::DEFAULT_SCRIPTS_DIR.to_string(),
        }
    }
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
        if self.api_key.is_empty() || self.connection_id.is_empty() || self.from_number.is_empty() {
            return Err(VoiceError::NotConfigured);
        }
        let body = json!({
            "to": to,
            "from": self.from_number,
            "connection_id": self.connection_id,
            "command_id": command_id,
            "webhook_url_method": "POST",
        });
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

/// Telnyx client failure. Messages carry status codes and short
/// server phrases only — never the API key, and never request
/// bodies (they hold phone numbers).
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

impl VoiceError {
    fn api(status: u16, body: &[u8]) -> Self {
        // Short server phrase only; bodies can echo request data.
        let text = String::from_utf8_lossy(body);
        let trimmed: String = text.chars().take(120).collect();
        VoiceError::Api {
            status,
            message: format!("telnyx status {status}: {}", trimmed.trim()),
        }
    }
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
    fn api_errors_carry_status_not_bodies() {
        let (base, _seen, handle) =
            stub(|_, _| (422, r#"{"errors":[{"detail":"bad number"}]}"#.to_string()));
        let err = client(&base).dial("+13605550100", "cmd-1").unwrap_err();
        assert!(matches!(err, VoiceError::Api { status: 422, .. }), "{err}");
        // The callee number must not leak into the error text.
        assert!(!err.to_string().contains("3605550100"), "{err}");
        handle.join().unwrap();
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
