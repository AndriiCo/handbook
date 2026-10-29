//! Script packs: prepared question lists staff read on pilot calls.
//!
//! Packs are repo TOML files under `crates/voice/scripts/` (spec
//! `docs/specs/voice-telnyx.md` §11.1), loaded at dial time. Inline
//! `questions[]` on the dial request win over any pack; when dial
//! names neither, the `default` pack fills in. A named pack that is
//! missing or invalid refuses the dial (422) — never an empty
//! question list on a live call.

use std::fmt;
use std::path::{Path, PathBuf};

/// Fallback pack stem: loaded when dial passes neither inline
/// questions nor a pack id.
pub const DEFAULT_PACK_ID: &str = "default";

/// `script_id` stored when the dial request carried its own list.
pub const INLINE_SCRIPT_ID: &str = "inline";

/// Default pack directory (repo-relative; the server runs from the
/// repo root, same as the `paths.*` static-asset convention).
pub const DEFAULT_SCRIPTS_DIR: &str = "crates/voice/scripts";

/// Max pack file size in bytes (stays reviewable in a diff).
pub const MAX_PACK_BYTES: usize = 16 * 1024;

/// Max questions per pack (and per dial).
pub const MAX_QUESTIONS: usize = 20;

/// Max chars per question, after trim.
pub const MAX_QUESTION_CHARS: usize = 280;

/// Max chars for the authority display name, after trim.
pub const MAX_AUTHORITY_CHARS: usize = 128;

/// A validated script pack: id, display authority, topics, questions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScriptPack {
    pub id: String,
    pub authority: String,
    pub topics: Vec<String>,
    pub questions: Vec<String>,
}

/// Pack load/validation failure. Reasons stay short — never pack
/// bytes — so the 422 response and audit row stay small.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    /// No pack by that stem (or the stem is not a safe file stem).
    Missing,
    /// File exceeds [`MAX_PACK_BYTES`].
    TooLarge,
    /// Bytes are not UTF-8 TOML with the §11.1 schema.
    Parse(String),
    /// Schema parsed but a §11.1 rule failed (reason names the rule).
    Invalid(String),
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptError::Missing => f.write_str("unknown script pack"),
            ScriptError::TooLarge => f.write_str("script pack exceeds 16 KiB"),
            ScriptError::Parse(why) | ScriptError::Invalid(why) => {
                write!(f, "invalid script pack: {why}")
            }
        }
    }
}

impl std::error::Error for ScriptError {}

/// Resolve the pack directory: the config override wins unless empty.
pub fn resolve_dir(cfg: &crate::VoiceConfig) -> PathBuf {
    if cfg.scripts_dir.trim().is_empty() {
        PathBuf::from(DEFAULT_SCRIPTS_DIR)
    } else {
        PathBuf::from(cfg.scripts_dir.trim())
    }
}

/// Kebab-case stem: lowercase ASCII, digits, hyphens; must start and
/// end alphanumeric (so it is always a safe single path segment —
/// `..` and `/` never pass).
pub fn is_kebab_case(stem: &str) -> bool {
    if stem.is_empty() || !stem.bytes().all(is_kebab_byte) {
        return false;
    }
    let bytes = stem.as_bytes();
    bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
}

fn is_kebab_byte(b: u8) -> bool {
    b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-'
}

/// Load and validate `dir/{stem}.toml`.
pub fn load_pack(dir: &Path, stem: &str) -> Result<ScriptPack, ScriptError> {
    if !is_kebab_case(stem) {
        return Err(ScriptError::Missing);
    }
    let bytes =
        std::fs::read(dir.join(format!("{stem}.toml"))).map_err(|_| ScriptError::Missing)?;
    parse_pack(&bytes, stem)
}

/// Validate raw pack bytes against the filename stem (§11.1 rules).
pub fn parse_pack(bytes: &[u8], stem: &str) -> Result<ScriptPack, ScriptError> {
    if bytes.len() > MAX_PACK_BYTES {
        return Err(ScriptError::TooLarge);
    }
    let text =
        std::str::from_utf8(bytes).map_err(|_| ScriptError::Parse("pack is not UTF-8".into()))?;
    let raw: RawPack = toml::from_str(text).map_err(|err| {
        let detail: String = err.to_string().chars().take(120).collect();
        ScriptError::Parse(format!("pack TOML: {}", detail.trim()))
    })?;
    raw.validate(stem)
}

#[derive(serde::Deserialize)]
struct RawPack {
    id: String,
    authority: String,
    topics: Vec<String>,
    questions: Vec<String>,
}

impl RawPack {
    fn validate(self, stem: &str) -> Result<ScriptPack, ScriptError> {
        if self.id != stem || !is_kebab_case(&self.id) {
            return Err(ScriptError::Invalid(
                "id must be kebab-case matching the filename stem".into(),
            ));
        }
        let authority = self.authority.trim().to_string();
        let chars = authority.chars().count();
        if chars == 0 || chars > MAX_AUTHORITY_CHARS {
            return Err(ScriptError::Invalid("authority must be 1-128 chars".into()));
        }
        if self.topics.is_empty() {
            return Err(ScriptError::Invalid(
                "topics must name at least one handbook topic".into(),
            ));
        }
        for topic in &self.topics {
            if contracts::topics::Topic::parse(topic).is_err() {
                return Err(ScriptError::Invalid(format!(
                    "unknown handbook topic: {topic}"
                )));
            }
        }
        if self.questions.is_empty() || self.questions.len() > MAX_QUESTIONS {
            return Err(ScriptError::Invalid("questions must be 1-20 items".into()));
        }
        let mut questions = Vec::with_capacity(self.questions.len());
        for question in &self.questions {
            let trimmed = question.trim();
            let chars = trimmed.chars().count();
            if chars == 0 || chars > MAX_QUESTION_CHARS {
                return Err(ScriptError::Invalid(
                    "each question must be 1-280 chars after trim".into(),
                ));
            }
            questions.push(trimmed.to_string());
        }
        Ok(ScriptPack {
            id: self.id,
            authority,
            topics: self.topics,
            questions,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack_toml(id: &str, authority: &str, topics: &str, questions: &str) -> String {
        format!(
            "id = \"{id}\"\nauthority = \"{authority}\"\ntopics = [{topics}]\nquestions = [{questions}]\n"
        )
    }

    fn valid_toml() -> String {
        pack_toml(
            "wa-auditor",
            "Fictional Harbor Records Office",
            "\"hours\", \"fees\"",
            "\"What are your walk-in hours?\", \"What is the per-document fee?\"",
        )
    }

    fn invalid_reason(err: ScriptError) -> String {
        match err {
            ScriptError::Invalid(why) => why,
            other => panic!("expected Invalid, got {other:?}"),
        }
    }

    #[test]
    fn valid_pack_parses_and_trims() {
        let toml = pack_toml(
            "wa-auditor",
            "  Fictional Harbor Records Office  ",
            "\"hours\"",
            "\"  Padded question?  \"",
        );
        let pack = parse_pack(toml.as_bytes(), "wa-auditor").unwrap();
        assert_eq!(pack.id, "wa-auditor");
        assert_eq!(pack.authority, "Fictional Harbor Records Office");
        assert_eq!(pack.topics, ["hours"]);
        assert_eq!(pack.questions, ["Padded question?"]);
    }

    #[test]
    fn id_must_match_stem() {
        let err = parse_pack(valid_toml().as_bytes(), "other-stem").unwrap_err();
        assert_eq!(
            invalid_reason(err),
            "id must be kebab-case matching the filename stem"
        );
    }

    #[test]
    fn id_must_be_kebab_case() {
        for id in ["WA-auditor", "wa_auditor", "wa auditor", "-wa", "wa-", ""] {
            let toml = pack_toml(id, "Office", "\"hours\"", "\"Q?\"");
            let stem = if id.is_empty() { "empty" } else { id };
            let err = parse_pack(toml.as_bytes(), stem).unwrap_err();
            assert!(
                matches!(err, ScriptError::Invalid(_)),
                "{id:?} should fail kebab-case: {err:?}"
            );
        }
        assert!(is_kebab_case("wa-secretary-of-state"));
        assert!(is_kebab_case("a1-b2"));
        assert!(!is_kebab_case("../escape"));
        assert!(!is_kebab_case("a/b"));
    }

    #[test]
    fn authority_bounds() {
        let over = "O".repeat(MAX_AUTHORITY_CHARS + 1);
        for (authority, ok) in [
            ("O", true),
            (&"O".repeat(MAX_AUTHORITY_CHARS), true),
            ("", false),
            ("   ", false),
            (over.as_str(), false),
        ] {
            let toml = pack_toml("wa-auditor", authority, "\"hours\"", "\"Q?\"");
            let parsed = parse_pack(toml.as_bytes(), "wa-auditor");
            assert_eq!(parsed.is_ok(), ok, "{authority:?}");
            if !ok {
                assert_eq!(
                    invalid_reason(parsed.unwrap_err()),
                    "authority must be 1-128 chars"
                );
            }
        }
    }

    #[test]
    fn topics_need_one_known_topic() {
        let toml = pack_toml("wa-auditor", "Office", "", "\"Q?\"");
        let err = parse_pack(toml.as_bytes(), "wa-auditor").unwrap_err();
        assert_eq!(
            invalid_reason(err),
            "topics must name at least one handbook topic"
        );

        let toml = pack_toml("wa-auditor", "Office", "\"hours\", \"bogus\"", "\"Q?\"");
        let err = parse_pack(toml.as_bytes(), "wa-auditor").unwrap_err();
        assert_eq!(invalid_reason(err), "unknown handbook topic: bogus");
    }

    #[test]
    fn every_frozen_topic_is_accepted() {
        for topic in contracts::topics::Topic::ALL {
            let toml = pack_toml(
                "wa-auditor",
                "Office",
                &format!("\"{}\"", topic.as_str()),
                "\"Q?\"",
            );
            assert!(
                parse_pack(toml.as_bytes(), "wa-auditor").is_ok(),
                "{topic:?}"
            );
        }
        assert_eq!(contracts::topics::Topic::ALL.len(), 25);
    }

    #[test]
    fn question_count_bounds() {
        let toml = pack_toml("wa-auditor", "Office", "\"hours\"", "");
        let err = parse_pack(toml.as_bytes(), "wa-auditor").unwrap_err();
        assert_eq!(invalid_reason(err), "questions must be 1-20 items");

        let many = (0..21).map(|i| format!("\"Q{i}?\"")).collect::<Vec<_>>();
        let toml = pack_toml("wa-auditor", "Office", "\"hours\"", &many.join(", "));
        let err = parse_pack(toml.as_bytes(), "wa-auditor").unwrap_err();
        assert_eq!(invalid_reason(err), "questions must be 1-20 items");

        let twenty = (0..20).map(|i| format!("\"Q{i}?\"")).collect::<Vec<_>>();
        let toml = pack_toml("wa-auditor", "Office", "\"hours\"", &twenty.join(", "));
        assert_eq!(
            parse_pack(toml.as_bytes(), "wa-auditor")
                .unwrap()
                .questions
                .len(),
            20
        );
    }

    #[test]
    fn question_length_bounds_after_trim() {
        let long = "Q".repeat(MAX_QUESTION_CHARS + 1);
        for (question, ok) in [
            ("Q?", true),
            (&"Q".repeat(MAX_QUESTION_CHARS), true),
            ("", false),
            ("   ", false),
            (long.as_str(), false),
        ] {
            let toml = pack_toml(
                "wa-auditor",
                "Office",
                "\"hours\"",
                &format!("\"{question}\""),
            );
            let parsed = parse_pack(toml.as_bytes(), "wa-auditor");
            assert_eq!(parsed.is_ok(), ok, "{question:?}");
            if !ok {
                assert_eq!(
                    invalid_reason(parsed.unwrap_err()),
                    "each question must be 1-280 chars after trim"
                );
            }
        }
    }

    #[test]
    fn oversize_pack_refused() {
        let filler = "Q".repeat(MAX_PACK_BYTES);
        let toml = pack_toml(
            "wa-auditor",
            "Office",
            "\"hours\"",
            &format!("\"{filler}\""),
        );
        assert!(toml.len() > MAX_PACK_BYTES);
        assert_eq!(
            parse_pack(toml.as_bytes(), "wa-auditor").unwrap_err(),
            ScriptError::TooLarge
        );
    }

    #[test]
    fn non_utf8_and_bad_toml_are_parse_errors() {
        assert!(matches!(
            parse_pack(&[0xff, 0xfe, 0x00], "wa-auditor").unwrap_err(),
            ScriptError::Parse(_)
        ));
        assert!(matches!(
            parse_pack(b"id = [unclosed", "wa-auditor").unwrap_err(),
            ScriptError::Parse(_)
        ));
        // Missing schema field (no questions key).
        assert!(matches!(
            parse_pack(
                b"id = \"wa-auditor\"\nauthority = \"O\"\ntopics = [\"hours\"]\n",
                "wa-auditor"
            )
            .unwrap_err(),
            ScriptError::Parse(_)
        ));
    }

    #[test]
    fn missing_file_and_unsafe_stem_are_missing() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
        assert_eq!(
            load_pack(&dir, "no-such-pack").unwrap_err(),
            ScriptError::Missing
        );
        assert_eq!(
            load_pack(&dir, "../escape").unwrap_err(),
            ScriptError::Missing
        );
        assert_eq!(load_pack(&dir, "").unwrap_err(), ScriptError::Missing);
    }

    #[test]
    fn checked_in_packs_load() {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("scripts");
        let default = load_pack(&dir, DEFAULT_PACK_ID).unwrap();
        assert_eq!(default.id, DEFAULT_PACK_ID);
        assert_eq!(default.questions.len(), 3);
        let example = load_pack(&dir, "synthetic-harbor-office").unwrap();
        assert_eq!(example.id, "synthetic-harbor-office");
        assert!(!example.questions.is_empty());
        // Morning-1 operational packs (real offices, our questions).
        for (stem, questions) in [
            ("wa-sos", 6),
            ("usdos-authentications", 6),
            ("egypt-embassy-dc", 6),
        ] {
            let pack = load_pack(&dir, stem).unwrap();
            assert_eq!(pack.id, stem);
            assert_eq!(pack.questions.len(), questions, "{stem}");
        }
    }

    #[test]
    fn resolve_dir_prefers_override() {
        let empty = crate::VoiceConfig::default();
        assert_eq!(resolve_dir(&empty), PathBuf::from(DEFAULT_SCRIPTS_DIR));
        let custom = crate::VoiceConfig {
            scripts_dir: "/tmp/packs".into(),
            ..crate::VoiceConfig::default()
        };
        assert_eq!(resolve_dir(&custom), PathBuf::from("/tmp/packs"));
    }
}
