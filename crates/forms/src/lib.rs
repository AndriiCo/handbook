//! P2 form templates: content-repo registry, server-side validator,
//! and submission renderers (PDF + HTML-print seam).
//!
//! Templates are content, not code
//! ([p2-forms-labels.md](../../../../docs/specs/p2-forms-labels.md) P2-F5):
//! JSON files under `forms/<authority>/<code>/v<n>.json` (interim
//! fallback: gitignored `store/forms/`). This crate holds the schema
//! types, the authoritative validator, and the renderers; it never
//! touches customer PII storage itself.
//!
//! Start at [`TemplateRegistry`], [`validate_submission`], and the
//! [`FormRenderer`] trait ([`PdfRenderer`], [`HtmlPrintRenderer`]).

pub mod acro;
pub mod registry;
pub mod render;
pub mod template;
pub mod validate;

pub use registry::{TemplateDiff, TemplateRegistry, diff_fields};
pub use render::{
    FormRenderer, HtmlPrintRenderer, PdfRenderer, RenderArtifact, RenderError, Submission,
};
pub use template::{
    FieldDef, FieldType, FormTemplate, PdfRect, ReplicaAcceptance, SignatureMode, TemplateError,
    TemplateStatus,
};
pub use validate::{FieldError, parse_money_cents, validate_submission};
