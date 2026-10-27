//! Per-authority form-template model (P2-F1, P2-F2).
//!
//! One [`FormTemplate`] per authority form edition. `version` is our
//! internal monotonic int; `authority_revision` is the authority's own
//! edition mark — never conflate the two.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Lifecycle of one template edition (P2-F1).
///
/// Exactly one `Active` per (`authority`, `code`) — enforced by
/// [`crate::TemplateRegistry::publish`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TemplateStatus {
    Draft,
    Active,
    NeedsReview,
    Superseded,
    Retired,
}

/// Whether the authority accepts our printed replica (P2-A1).
///
/// Unknown is the default for new templates: submission is allowed but
/// flagged `replica_unconfirmed` until staff research lands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ReplicaAcceptance {
    Yes,
    No,
    #[default]
    Unknown,
}

/// How the customer signs (P2-S2).
///
/// P2 renders [`SignatureMode::WetInk`] only: the PDF carries an empty
/// signature + date line and the customer signs in ink after printing.
/// The drawn variants exist so templates can name the intent; the
/// renderer rejects them with a typed error until drawn support ships.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum SignatureMode {
    #[default]
    WetInk,
    DrawnOptional,
    DrawnRequired,
}

/// Value shape of one template field (P2-F2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum FieldType {
    Text,
    Textarea,
    Email,
    Tel,
    Date,
    Number,
    Money,
    Select,
    Radio,
    Checkbox,
    AddressBlock,
}

/// Absolute-position rect for one field on the rendered form (P2-R2).
///
/// `x_mm`/`y_mm` are millimetres from the page's top-left corner
/// (natural for template authors measuring against the official
/// edition); renderers convert to PDF bottom-left origin. `page` is
/// 1-based. US government forms print on US Letter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PdfRect {
    pub page: u32,
    pub x_mm: f32,
    pub y_mm: f32,
    pub w_mm: f32,
    pub h_mm: f32,
    pub font_size_pt: f32,
    #[serde(default)]
    pub multiline: bool,
}

/// One fillable field in a template's ordered schema (P2-F2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    pub key: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<String>,
    #[serde(rename = "type")]
    pub field_type: FieldType,
    #[serde(default)]
    pub required: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_len: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prefill_topic: Option<String>,
    pub pdf: PdfRect,
}

/// One authority form edition: schema + layout + acceptance flags.
///
/// Field names mirror the P2-D1 row; the content-repo JSON file is the
/// authoritative copy and the DB row is the indexed cache.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FormTemplate {
    pub authority: String,
    pub code: String,
    pub title: String,
    pub version: u32,
    pub authority_revision: String,
    pub status: TemplateStatus,
    pub fields: Vec<FieldDef>,
    pub accepts_replica: ReplicaAcceptance,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accepts_replica_capture_id: Option<i64>,
    #[serde(default)]
    pub accepts_drawn_signature: bool,
    #[serde(default)]
    pub signature_mode: SignatureMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source_capture_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fact_ids: Vec<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub supersedes_id: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_verified_at: Option<i64>,
    #[serde(default = "default_verify_cadence_days")]
    pub verify_cadence_days: u32,
    #[serde(default)]
    pub created_by: String,
    #[serde(default)]
    pub created_at: i64,
}

fn default_verify_cadence_days() -> u32 {
    90
}

impl FormTemplate {
    /// Check internal consistency: unique non-empty keys, compilable
    /// patterns, sane rects, options present for choice types.
    ///
    /// # Errors
    ///
    /// Returns [`TemplateError`] naming the first defect found.
    pub fn validate_schema(&self) -> Result<(), TemplateError> {
        if self.code.trim().is_empty() {
            return Err(TemplateError::EmptyCode);
        }
        if self.version == 0 {
            return Err(TemplateError::BadVersion);
        }
        let mut seen = std::collections::BTreeSet::new();
        for field in &self.fields {
            if field.key.trim().is_empty() {
                return Err(TemplateError::EmptyKey);
            }
            if !seen.insert(field.key.clone()) {
                return Err(TemplateError::DuplicateKey(field.key.clone()));
            }
            if let Some(pattern) = &field.pattern {
                regex::Regex::new(&format!("^(?:{pattern})$"))
                    .map_err(|_| TemplateError::BadPattern(field.key.clone()))?;
            }
            let rect = &field.pdf;
            if rect.page == 0
                || !rect.x_mm.is_finite()
                || !rect.y_mm.is_finite()
                || rect.w_mm <= 0.0
                || rect.h_mm <= 0.0
                || rect.font_size_pt <= 0.0
            {
                return Err(TemplateError::BadRect(field.key.clone()));
            }
            if matches!(field.field_type, FieldType::Select | FieldType::Radio)
                && field.options.is_empty()
            {
                return Err(TemplateError::MissingOptions(field.key.clone()));
            }
        }
        Ok(())
    }

    /// True when the re-verification SLA has expired at `now_unix`
    /// (P2-F6): the template must flip `active → needs_review`.
    pub fn sla_expired(&self, now_unix: i64) -> bool {
        match self.last_verified_at {
            None => true,
            Some(at) => {
                now_unix.saturating_sub(at)
                    > i64::from(self.verify_cadence_days).saturating_mul(86_400)
            }
        }
    }
}

/// [`FormTemplate::validate_schema`] failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TemplateError {
    EmptyCode,
    BadVersion,
    EmptyKey,
    DuplicateKey(String),
    BadPattern(String),
    BadRect(String),
    MissingOptions(String),
}

impl fmt::Display for TemplateError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TemplateError::EmptyCode => write!(f, "template code is empty"),
            TemplateError::BadVersion => write!(f, "template version must be >= 1"),
            TemplateError::EmptyKey => write!(f, "field key is empty"),
            TemplateError::DuplicateKey(key) => write!(f, "duplicate field key: {key}"),
            TemplateError::BadPattern(key) => {
                write!(f, "field {key}: pattern does not compile")
            }
            TemplateError::BadRect(key) => write!(f, "field {key}: bad pdf rect"),
            TemplateError::MissingOptions(key) => {
                write!(f, "field {key}: choice type needs options")
            }
        }
    }
}

impl std::error::Error for TemplateError {}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) fn sample_template() -> FormTemplate {
        FormTemplate {
            authority: "usa-wa-sos".to_string(),
            code: "wa-apostille-request".to_string(),
            title: "WA Apostille Request".to_string(),
            version: 1,
            authority_revision: "Rev 06/2025".to_string(),
            status: TemplateStatus::Draft,
            fields: vec![FieldDef {
                key: "full_name".to_string(),
                label: "Full name".to_string(),
                help: None,
                field_type: FieldType::Text,
                required: true,
                max_len: Some(80),
                pattern: None,
                options: Vec::new(),
                prefill_topic: None,
                pdf: PdfRect {
                    page: 1,
                    x_mm: 20.0,
                    y_mm: 60.0,
                    w_mm: 170.0,
                    h_mm: 8.0,
                    font_size_pt: 11.0,
                    multiline: false,
                },
            }],
            accepts_replica: ReplicaAcceptance::Unknown,
            accepts_replica_capture_id: None,
            accepts_drawn_signature: false,
            signature_mode: SignatureMode::WetInk,
            source_capture_id: None,
            fact_ids: Vec::new(),
            supersedes_id: None,
            last_verified_at: None,
            verify_cadence_days: 90,
            created_by: "staff".to_string(),
            created_at: 1_700_000_000,
        }
    }

    #[test]
    fn schema_accepts_sample() {
        sample_template().validate_schema().unwrap();
    }

    #[test]
    fn schema_rejects_defects() {
        let mut template = sample_template();
        template.fields.push(template.fields[0].clone());
        assert_eq!(
            template.validate_schema().unwrap_err(),
            TemplateError::DuplicateKey("full_name".to_string())
        );

        let mut template = sample_template();
        template.fields[0].pattern = Some("([".to_string());
        assert_eq!(
            template.validate_schema().unwrap_err(),
            TemplateError::BadPattern("full_name".to_string())
        );

        let mut template = sample_template();
        template.fields[0].pdf.page = 0;
        assert_eq!(
            template.validate_schema().unwrap_err(),
            TemplateError::BadRect("full_name".to_string())
        );

        let mut template = sample_template();
        template.fields[0].field_type = FieldType::Select;
        assert_eq!(
            template.validate_schema().unwrap_err(),
            TemplateError::MissingOptions("full_name".to_string())
        );
    }

    #[test]
    fn sla_expiry() {
        let mut template = sample_template();
        assert!(template.sla_expired(1_800_000_000));
        template.last_verified_at = Some(1_799_000_000);
        assert!(!template.sla_expired(1_799_000_000 + 10));
        assert!(template.sla_expired(1_799_000_000 + 91 * 86_400));
    }
}
