//! Agent interface: the request/response shapes and the
//! [`ModelClient`] trait. Implementations (stub, live) live in
//! `handbook-agent`; every other consumer programs to this trait.

use std::fmt;

/// Request to draft structured records from evidence text.
#[derive(Debug, Clone, PartialEq)]
pub struct DraftRequest {
    pub authority_slug: String,
    pub prompt_version: &'static str,
    pub input_text: String,
    pub max_records: u32,
    /// Provenance id; `UNATTRIBUTED_CAPTURE_ID` when hand-fed.
    pub capture_id: i64,
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
    /// Model's own per-response summary (quarantined: served only as
    /// `reasoning_untrusted`, never a decision input). May be empty.
    pub summary: String,
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

    /// Model id for `fact.created_by` attribution (e.g. `"muse-spark-1.3"`).
    /// Test fakes keep the default; it never reaches storage for them
    /// because their records are rejected or their calls fail.
    fn model_id(&self) -> &'static str {
        "unknown"
    }
}
