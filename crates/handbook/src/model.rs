//! Model-client trait and implementations for the analysis agent.
//!
//! Runner code depends only on [`ModelClient`]. [`StubClient`] serves
//! deterministic fixtures for tests and offline dev; [`MetaRestClient`]
//! is the live client skeleton (not yet wired to HTTP).

use std::fmt;

/// Request to draft structured records from evidence text.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftRequest {
    pub authority_slug: String,
    pub prompt_version: &'static str,
    pub input_text: String,
    pub max_records: u32,
}

/// Byte-exact citation into a source document.
#[derive(Debug, Clone, PartialEq)]
pub struct CitationRef {
    pub doc_id: String,
    pub byte_start: u32,
    pub byte_end: u32,
}

/// One model-proposed record (always `proposed`, never auto-accepted).
#[derive(Debug, Clone, PartialEq)]
pub struct ProposedRecord {
    pub topic: String,
    pub field: String,
    pub value: String,
    pub citation: CitationRef,
    pub confidence: f32,
}

/// Token usage for one draft call (for audit/cost logging).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// Model reply: proposed records plus usage.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftResponse {
    pub records: Vec<ProposedRecord>,
    pub usage: Usage,
}

/// Errors from a [`ModelClient`] draft call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModelError {
    Transport(String),
    BadOutput(String),
    BudgetExceeded,
    Auth(String),
}

impl fmt::Display for ModelError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ModelError::Transport(msg) => write!(f, "model transport error: {msg}"),
            ModelError::BadOutput(msg) => write!(f, "model bad output: {msg}"),
            ModelError::BudgetExceeded => write!(f, "model budget exceeded"),
            ModelError::Auth(msg) => write!(f, "model auth error: {msg}"),
        }
    }
}

impl std::error::Error for ModelError {}

/// Draft structured records from evidence text.
pub trait ModelClient {
    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError>;
}

/// Deterministic fixture client for tests and offline dev.
///
/// Returns the same fixed records for every call (capped at
/// `req.max_records`); citations reference `doc_id` `"fixture"`.
#[derive(Debug, Clone, Default)]
pub struct StubClient;

impl ModelClient for StubClient {
    fn draft_records(&self, req: &DraftRequest) -> Result<DraftResponse, ModelError> {
        let records = vec![
            ProposedRecord {
                topic: "address".to_string(),
                field: "po_box".to_string(),
                value: "PO Box 40220".to_string(),
                citation: CitationRef {
                    doc_id: "fixture".to_string(),
                    byte_start: 0,
                    byte_end: 12,
                },
                confidence: 0.9,
            },
            ProposedRecord {
                topic: "hours".to_string(),
                field: "weekday".to_string(),
                value: "Mon-Fri 9-5".to_string(),
                citation: CitationRef {
                    doc_id: "fixture".to_string(),
                    byte_start: 13,
                    byte_end: 24,
                },
                confidence: 0.8,
            },
        ];
        let records = records.into_iter().take(req.max_records as usize).collect();
        Ok(DraftResponse {
            records,
            usage: Usage {
                input_tokens: 0,
                output_tokens: 0,
            },
        })
    }
}

/// Live Meta Models API client (skeleton: not yet wired to HTTP).
///
/// Auth gotcha: Model API calls need a Model API key (`META_API_KEY`
/// env). The OAuth2 handshake cannot mint Model API tokens, so an
/// auth-code exchange is unusable for Model API auth — key auth only.
/// See `docs/meta-api.md`.
///
/// `Debug` is hand-written (M3): the key never appears in debug output,
/// so a stray `{:?}` in an error or log path cannot leak it.
#[derive(Clone)]
#[allow(dead_code)] // fields wire up once HTTP is implemented
pub struct MetaRestClient {
    base_url: String,
    api_key: String,
}

impl fmt::Debug for MetaRestClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MetaRestClient")
            .field("base_url", &self.base_url)
            .field("api_key", &"<redacted>")
            .finish()
    }
}

impl MetaRestClient {
    pub fn new(base_url: String, api_key: String) -> Self {
        Self { base_url, api_key }
    }
}

impl ModelClient for MetaRestClient {
    fn draft_records(&self, _req: &DraftRequest) -> Result<DraftResponse, ModelError> {
        Err(ModelError::Transport(
            "live calls not implemented".to_string(),
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_request() -> DraftRequest {
        DraftRequest {
            authority_slug: "wa-sos".to_string(),
            prompt_version: "aa-prompt-v1",
            input_text: "PO Box 40220".to_string(),
            max_records: 4,
        }
    }

    #[test]
    fn stub_is_deterministic() {
        let client = StubClient;
        let req = sample_request();
        let first = client.draft_records(&req).expect("stub drafts");
        let second = client.draft_records(&req).expect("stub drafts");
        assert_eq!(first, second);
        assert!(!first.records.is_empty());
        assert!(
            first.records.iter().all(|r| r.citation.doc_id == "fixture"),
            "stub cites fixture doc only"
        );
    }

    #[test]
    fn stub_respects_max_records() {
        let client = StubClient;
        let mut req = sample_request();
        req.max_records = 1;
        let resp = client.draft_records(&req).expect("stub drafts");
        assert_eq!(resp.records.len(), 1);
    }

    #[test]
    fn meta_client_new_stores_config() {
        let client =
            MetaRestClient::new("https://api.meta.ai/v1".to_string(), "sekrit".to_string());
        assert_eq!(client.base_url, "https://api.meta.ai/v1");
        assert_eq!(client.api_key, "sekrit");
    }

    #[test]
    fn meta_client_debug_redacts_key() {
        let client =
            MetaRestClient::new("https://api.meta.ai/v1".to_string(), "sekrit".to_string());
        let shown = format!("{client:?}");
        assert!(!shown.contains("sekrit"), "got: {shown}");
        assert!(shown.contains("api_key"), "got: {shown}");
    }

    #[test]
    fn meta_client_draft_is_honest_err() {
        let client = MetaRestClient::new("https://api.meta.ai/v1".to_string(), "k".to_string());
        let err = client
            .draft_records(&sample_request())
            .expect_err("not implemented");
        assert!(matches!(err, ModelError::Transport(_)));
    }
}
