//! F8 upload intake: rights grant + quarantine (app-features F8).
//!
//! Upload requires accepting the rights grant (instruction photos
//! become Andrii Co. property; grant version + timestamp are recorded
//! on the capture) — refusal blocks the upload. Accepted captures
//! land quarantined with `community` trust and keep EXIF in the
//! original; F9 derivatives strip it.

use std::fmt;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::policy::{MediaPolicy, MediaSource, PolicyError, check};

/// Largest accepted upload: 32 MiB covers phone photos and short
/// clips; anything bigger is a staff-assisted ingest.
pub const MAX_UPLOAD_BYTES: usize = 32 * 1024 * 1024;

/// Current rights-grant text version. Uploads citing any other
/// version must re-accept.
pub const GRANT_VERSION: &str = "andrii-co-media-grant-v1";

/// Capture kind for the ingested bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum MediaKind {
    Photo,
    Video,
    Comment,
}

/// Trust tier of a fresh upload: always community (F8).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Trust {
    Community,
}

/// The upload-time rights grant: version + acceptance time.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RightsGrant {
    pub version: String,
    pub accepted_at_unix: i64,
}

/// An accepted upload, ready to persist as a quarantined capture.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intake {
    pub filename: String,
    pub content_type: String,
    pub media_kind: MediaKind,
    pub sha256_hex: String,
    pub bytes_len: usize,
    pub quarantined: bool,
    pub trust: Trust,
    pub exif_preserved: bool,
    pub grant_version: String,
    pub grant_accepted_at: i64,
}

/// Validate an upload and record its grant.
///
/// # Errors
///
/// Returns [`UploadError`] when the terms gate blocks real media, the
/// grant is missing or stale, or the file is rejected (type, size,
/// filename).
pub fn intake_upload(
    policy: &MediaPolicy,
    source: MediaSource,
    filename: &str,
    content_type: &str,
    bytes: &[u8],
    grant: Option<&RightsGrant>,
    now_unix: i64,
) -> Result<Intake, UploadError> {
    check(policy, source)?;
    let grant = grant.ok_or(UploadError::GrantRequired)?;
    if grant.version != GRANT_VERSION {
        return Err(UploadError::GrantMismatch);
    }
    let media_kind = classify(content_type).ok_or_else(|| UploadError::UnsupportedType {
        content_type: content_type.to_string(),
    })?;
    if bytes.len() > MAX_UPLOAD_BYTES {
        return Err(UploadError::TooLarge {
            max_bytes: MAX_UPLOAD_BYTES,
        });
    }
    if bytes.is_empty() {
        return Err(UploadError::Empty);
    }
    let clean = clean_filename(filename).ok_or(UploadError::BadFilename)?;
    let _ = now_unix;
    Ok(Intake {
        filename: clean,
        content_type: content_type.to_string(),
        media_kind,
        sha256_hex: format!("{:x}", Sha256::digest(bytes)),
        bytes_len: bytes.len(),
        quarantined: true,
        trust: Trust::Community,
        exif_preserved: true,
        grant_version: grant.version.clone(),
        grant_accepted_at: grant.accepted_at_unix,
    })
}

fn classify(content_type: &str) -> Option<MediaKind> {
    match content_type.to_lowercase().as_str() {
        "image/jpeg" | "image/png" | "image/webp" => Some(MediaKind::Photo),
        "video/mp4" | "video/quicktime" => Some(MediaKind::Video),
        "text/plain" => Some(MediaKind::Comment),
        _ => None,
    }
}

/// Basename only: no directories, no NUL, non-empty after trim.
fn clean_filename(filename: &str) -> Option<String> {
    let base = filename.rsplit(['/', '\\']).next()?.trim();
    if base.is_empty() || base == "." || base == ".." || base.contains('\0') {
        return None;
    }
    Some(base.to_string())
}

/// [`intake_upload`] failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum UploadError {
    TermsNotRead,
    GrantRequired,
    GrantMismatch,
    UnsupportedType { content_type: String },
    TooLarge { max_bytes: usize },
    Empty,
    BadFilename,
}

impl fmt::Display for UploadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            UploadError::TermsNotRead => write!(f, "{0}", PolicyError::TermsNotRead),
            UploadError::GrantRequired => {
                write!(f, "upload requires accepting the rights grant")
            }
            UploadError::GrantMismatch => {
                write!(f, "rights grant version is stale, re-accept to upload")
            }
            UploadError::UnsupportedType { content_type } => {
                write!(f, "unsupported upload type: {content_type}")
            }
            UploadError::TooLarge { max_bytes } => {
                write!(f, "upload exceeds the {max_bytes} byte limit")
            }
            UploadError::Empty => write!(f, "upload is empty"),
            UploadError::BadFilename => write!(f, "filename is invalid"),
        }
    }
}

impl std::error::Error for UploadError {}

impl From<PolicyError> for UploadError {
    fn from(_: PolicyError) -> Self {
        UploadError::TermsNotRead
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grant() -> RightsGrant {
        RightsGrant {
            version: GRANT_VERSION.to_string(),
            accepted_at_unix: 1_791_849_600,
        }
    }

    #[test]
    fn happy_path_quarantines() {
        let intake = intake_upload(
            &MediaPolicy::synthetic_only(),
            MediaSource::SyntheticFixture,
            "route.jpg",
            "image/jpeg",
            b"fake-jpeg-bytes",
            Some(&grant()),
            1_791_849_600,
        )
        .unwrap();
        assert_eq!(intake.media_kind, MediaKind::Photo);
        assert!(intake.quarantined);
        assert_eq!(intake.trust, Trust::Community);
        assert!(intake.exif_preserved);
        assert_eq!(intake.sha256_hex.len(), 64);
        assert_eq!(intake.grant_version, GRANT_VERSION);
    }

    #[test]
    fn refusals_block_upload() {
        let policy = MediaPolicy::synthetic_only();
        let source = MediaSource::SyntheticFixture;
        // Missing grant.
        assert_eq!(
            intake_upload(&policy, source, "a.jpg", "image/jpeg", b"x", None, 0).unwrap_err(),
            UploadError::GrantRequired
        );
        // Stale grant.
        let stale = RightsGrant {
            version: "v0".to_string(),
            accepted_at_unix: 0,
        };
        assert_eq!(
            intake_upload(
                &policy,
                source,
                "a.jpg",
                "image/jpeg",
                b"x",
                Some(&stale),
                0
            )
            .unwrap_err(),
            UploadError::GrantMismatch
        );
        // Bad type, empty, bad filename.
        assert!(matches!(
            intake_upload(
                &policy,
                source,
                "a.exe",
                "application/x-exe",
                b"x",
                Some(&grant()),
                0
            )
            .unwrap_err(),
            UploadError::UnsupportedType { .. }
        ));
        assert_eq!(
            intake_upload(
                &policy,
                source,
                "a.jpg",
                "image/jpeg",
                b"",
                Some(&grant()),
                0
            )
            .unwrap_err(),
            UploadError::Empty
        );
        assert_eq!(
            intake_upload(&policy, source, "..", "image/jpeg", b"x", Some(&grant()), 0)
                .unwrap_err(),
            UploadError::BadFilename
        );
        // A path uploads under its basename.
        let intake = intake_upload(
            &policy,
            source,
            "photos/../route.jpg",
            "image/jpeg",
            b"x",
            Some(&grant()),
            0,
        )
        .unwrap();
        assert_eq!(intake.filename, "route.jpg");
        // Real media before the terms read.
        assert_eq!(
            intake_upload(
                &policy,
                MediaSource::RealUpload,
                "a.jpg",
                "image/jpeg",
                b"x",
                Some(&grant()),
                0
            )
            .unwrap_err(),
            UploadError::TermsNotRead
        );
    }

    #[test]
    fn oversize_is_rejected() {
        let big = vec![0u8; MAX_UPLOAD_BYTES + 1];
        assert!(matches!(
            intake_upload(
                &MediaPolicy::synthetic_only(),
                MediaSource::SyntheticFixture,
                "big.mp4",
                "video/mp4",
                &big,
                Some(&grant()),
                0
            )
            .unwrap_err(),
            UploadError::TooLarge { .. }
        ));
    }
}
