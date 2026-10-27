//! F8/F9/P1 media: upload intake, SAM redaction, Voice transcripts.
//!
//! Uploads arrive with an upload-time rights grant (instruction
//! photos become Andrii Co. property — decided prompt 008), stay
//! quarantined `community` captures until F4 moderation, and never
//! render publicly without an F9 redacted derivative. Transcription
//! runs through Muse Voice Transcribe's file endpoint; the
//! transcript annotation model backs the Day-3 fix UI.
//!
//! Gating (specs README G3): [`MediaPolicy`] keeps the slice
//! synthetic-only until the Meta API data-handling terms are read —
//! real uploads are rejected before any byte reaches a Model API.
//! Meta Models API is an approved processor for customer media once
//! that gate opens.
//!
//! Start at [`MediaPolicy`], [`intake_upload`], the [`Redactor`]
//! trait ([`MetaSamClient`], [`StubRedactor`]), [`VoiceFileClient`],
//! and [`apply_fix`].

pub mod policy;
pub mod redact;
pub mod transcribe;
pub mod transcript;
pub mod transport;
pub mod upload;

pub use policy::{MediaPolicy, MediaSource, PolicyError, check};
pub use redact::{
    MetaSamClient, RedactBox, RedactError, RedactTarget, Redaction, Redactor, StubRedactor,
};
pub use transcribe::{TranscribeError, Transcript, TranscriptTurn, VoiceFileClient};
pub use transcript::{Annotation, AnnotationKind, TranscriptError, apply_fix};
pub use transport::{HttpResponse, HttpTransport, MultipartFile, TransportError, UreqTransport};
pub use upload::{
    GRANT_VERSION, Intake, MAX_UPLOAD_BYTES, MediaKind, RightsGrant, Trust, UploadError,
    intake_upload,
};
