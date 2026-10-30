//! Analysis-agent helpers: prompt building, response validation, redaction.
//!
//! Pure functions only — no DB access. The validator enforces
//! byte-exact citations against source texts.

use std::collections::HashMap;

use contracts::model::{DraftRequest, DraftResponse};

/// Fill `{{slug}}`, `{{input}}`, `{{max_records}}`, `{{doc_ids}}` in a
/// prompt template.
///
/// `input_text` is substituted last so evidence text is inserted
/// verbatim (a literal `{{slug}}` inside the evidence is not expanded).
pub fn build_prompt(template: &str, req: &DraftRequest) -> String {
    template
        .replace("{{max_records}}", &req.max_records.to_string())
        .replace("{{slug}}", &req.authority_slug)
        .replace("{{doc_ids}}", &req.doc_ids.join(", "))
        .replace("{{input}}", &req.input_text)
}

/// One response-validation failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidationError {
    pub record_index: usize,
    pub kind: ValidationErrorKind,
}

/// Machine-checkable validation failure kinds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationErrorKind {
    UnknownTopic,
    MissingDoc,
    BadRange,
    BadConfidence,
    /// Cited quote is shorter than [`MIN_QUOTE_CHARS`] characters.
    ShortQuote,
    /// The record value is not found inside its cited quote.
    ValueNotInQuote,
}

/// Minimum cited-quote length in characters: a shorter slice cannot
/// carry a verifiable claim (A1: `quote.trim().len() >= 8`).
pub const MIN_QUOTE_CHARS: usize = 8;

/// Normalize evidence text for containment checks: lowercase with all
/// whitespace runs collapsed to single spaces.
pub fn normalize(text: &str) -> String {
    text.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

/// Validate a draft response against sources and the frozen topic enum.
///
/// `sources` maps `doc_id` to source text. Citation byte ranges must be
/// in bounds and on char boundaries; confidence must be finite and in
/// `0.0..=1.0`. Returns every failure, not just the first.
pub fn validate_response(
    resp: &DraftResponse,
    sources: &HashMap<String, String>,
    allowed_topics: &[&str],
) -> Result<(), Vec<ValidationError>> {
    let mut errors = Vec::new();
    for (record_index, record) in resp.records.iter().enumerate() {
        if !allowed_topics.contains(&record.topic.as_str()) {
            errors.push(ValidationError {
                record_index,
                kind: ValidationErrorKind::UnknownTopic,
            });
        }
        match sources.get(&record.citation.doc_id) {
            None => errors.push(ValidationError {
                record_index,
                kind: ValidationErrorKind::MissingDoc,
            }),
            Some(source) => {
                let start = record.citation.byte_start as usize;
                let end = record.citation.byte_end as usize;
                // m11: empty ranges cite nothing — a citation must
                // cover at least one byte.
                let in_bounds = start < end && end <= source.len();
                let on_boundary =
                    in_bounds && source.is_char_boundary(start) && source.is_char_boundary(end);
                if !on_boundary {
                    errors.push(ValidationError {
                        record_index,
                        kind: ValidationErrorKind::BadRange,
                    });
                } else {
                    // Offsets are server-side evidence: the quote must
                    // be long enough to verify and must contain the
                    // record's own value, else the model is paraphrasing
                    // outside its citation.
                    let quote = &source[start..end];
                    if quote.chars().count() < MIN_QUOTE_CHARS {
                        errors.push(ValidationError {
                            record_index,
                            kind: ValidationErrorKind::ShortQuote,
                        });
                    } else if !normalize(quote).contains(&normalize(&record.value)) {
                        errors.push(ValidationError {
                            record_index,
                            kind: ValidationErrorKind::ValueNotInQuote,
                        });
                    }
                }
            }
        }
        if !record.confidence.is_finite() || !(0.0..=1.0).contains(&record.confidence) {
            errors.push(ValidationError {
                record_index,
                kind: ValidationErrorKind::BadConfidence,
            });
        }
    }
    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors)
    }
}

/// Replace ignored byte ranges in `text` with `[...]`, merging overlaps.
///
/// Ranges may be unsorted or overlapping; out-of-bounds ends are
/// clamped, inverted pairs are swapped, and edges snap outward to char
/// boundaries so output never splits a character.
pub fn redact_ignored(text: &str, ranges: &[(usize, usize)]) -> String {
    let mut spans: Vec<(usize, usize)> = ranges
        .iter()
        .map(|&(a, b)| (a.min(b), a.max(b)))
        .map(|(s, e)| (s.min(text.len()), e.min(text.len())))
        .map(|(s, e)| (floor_boundary(text, s), ceil_boundary(text, e)))
        .filter(|(s, e)| s < e)
        .collect();
    spans.sort();
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    for (start, end) in spans {
        if start >= cursor {
            out.push_str(&text[cursor..start]);
            out.push_str("[...]");
            cursor = end;
        } else if end > cursor {
            cursor = end;
        }
    }
    out.push_str(&text[cursor..]);
    out
}

fn floor_boundary(text: &str, mut i: usize) -> usize {
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

fn ceil_boundary(text: &str, mut i: usize) -> usize {
    while i < text.len() && !text.is_char_boundary(i) {
        i += 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use contracts::model::{CitationRef, ProposedRecord, Usage};

    fn record(topic: &str, doc_id: &str, start: u32, end: u32, confidence: f32) -> ProposedRecord {
        ProposedRecord {
            topic: topic.to_string(),
            field: "value".to_string(),
            // Matches the "PO Box 40220" source texts below, so these
            // fixtures exercise their named failure without tripping
            // the value-in-quote check incidentally.
            value: "PO Box 40220".to_string(),
            citation: CitationRef {
                doc_id: doc_id.to_string(),
                byte_start: start,
                byte_end: end,
            },
            confidence,
        }
    }

    fn response(records: Vec<ProposedRecord>) -> DraftResponse {
        DraftResponse {
            records,
            usage: Usage {
                input_tokens: 1,
                output_tokens: 1,
            },
            summary: String::new(),
        }
    }

    #[test]
    fn build_prompt_substitutes_placeholders() {
        let req = DraftRequest {
            authority_slug: "wa-sos".to_string(),
            prompt_version: "aa-prompt-v1",
            input_text: "evidence here".to_string(),
            max_records: 3,
            capture_id: 0,
            doc_ids: vec!["fixture".to_string()],
        };
        let out = build_prompt("AUTH={{slug}} MAX={{max_records}} IN={{input}}", &req);
        assert_eq!(out, "AUTH=wa-sos MAX=3 IN=evidence here");
    }

    #[test]
    fn build_prompt_names_admissible_doc_ids() {
        let req = DraftRequest {
            authority_slug: "wa-sos".to_string(),
            prompt_version: "aa-prompt-v1",
            input_text: "evidence here".to_string(),
            max_records: 3,
            capture_id: 0,
            doc_ids: vec!["fixture".to_string(), "sidecar".to_string()],
        };
        let out = build_prompt("DOCS: {{doc_ids}}", &req);
        assert_eq!(out, "DOCS: fixture, sidecar");
    }

    #[test]
    fn build_prompt_inserts_evidence_verbatim() {
        // Evidence is substituted LAST, so placeholder-looking text in
        // the evidence is never expanded (anti-injection ordering).
        let req = DraftRequest {
            authority_slug: "wa".to_string(),
            prompt_version: "aa-prompt-v1",
            input_text: "{{slug}} {{max_records}}".to_string(),
            max_records: 3,
            capture_id: 0,
            doc_ids: vec![],
        };
        let out = build_prompt("AUTH={{slug}} IN={{input}}", &req);
        assert_eq!(out, "AUTH=wa IN={{slug}} {{max_records}}");
    }

    #[test]
    fn validator_accepts_good_response() {
        let mut sources = HashMap::new();
        sources.insert("d1".to_string(), "PO Box 40220".to_string());
        let resp = response(vec![record("address", "d1", 0, 12, 0.9)]);
        assert_eq!(
            validate_response(&resp, &sources, &["address", "hours"]),
            Ok(())
        );
    }

    #[test]
    fn validator_rejects_bad_topic() {
        let mut sources = HashMap::new();
        sources.insert("d1".to_string(), "PO Box 40220".to_string());
        let resp = response(vec![record("mailing_address", "d1", 0, 12, 0.9)]);
        let errs = validate_response(&resp, &sources, &["address"]).expect_err("bad topic");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].record_index, 0);
        assert_eq!(errs[0].kind, ValidationErrorKind::UnknownTopic);
    }

    #[test]
    fn validator_rejects_missing_doc() {
        let sources = HashMap::new();
        let resp = response(vec![record("address", "nope", 0, 1, 0.5)]);
        let errs = validate_response(&resp, &sources, &["address"]).expect_err("missing doc");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ValidationErrorKind::MissingDoc);
    }

    #[test]
    fn validator_rejects_out_of_range_bytes() {
        let mut sources = HashMap::new();
        sources.insert("d1".to_string(), "short".to_string());
        let resp = response(vec![record("address", "d1", 0, 99, 0.5)]);
        let errs = validate_response(&resp, &sources, &["address"]).expect_err("out of range");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ValidationErrorKind::BadRange);
    }

    #[test]
    fn validator_rejects_split_char_boundary() {
        // 'e-acute' in "Montréal" occupies bytes 5..7.
        let mut sources = HashMap::new();
        sources.insert("d1".to_string(), "Montréal Bystřina".to_string());
        assert!(!sources["d1"].is_char_boundary(6));
        let resp = response(vec![record("address", "d1", 5, 6, 0.5)]);
        let errs = validate_response(&resp, &sources, &["address"]).expect_err("split char");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ValidationErrorKind::BadRange);
    }

    #[test]
    fn validator_rejects_empty_range() {
        let mut sources = HashMap::new();
        sources.insert("d1".to_string(), "PO Box 40220".to_string());
        let resp = response(vec![record("address", "d1", 5, 5, 0.5)]);
        let errs = validate_response(&resp, &sources, &["address"]).expect_err("empty range");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ValidationErrorKind::BadRange);
    }

    #[test]
    fn validator_rejects_bad_confidence() {
        let mut sources = HashMap::new();
        sources.insert("d1".to_string(), "PO Box 40220".to_string());
        for bad in [f32::NAN, 1.5, -0.1, f32::INFINITY] {
            let resp = response(vec![record("address", "d1", 0, 12, bad)]);
            let errs =
                validate_response(&resp, &sources, &["address"]).expect_err("bad confidence");
            assert_eq!(errs.len(), 1, "conf={bad}");
            assert_eq!(
                errs[0].kind,
                ValidationErrorKind::BadConfidence,
                "conf={bad}"
            );
        }
    }

    #[test]
    fn validator_returns_every_failure() {
        let mut sources = HashMap::new();
        sources.insert("d1".to_string(), "PO Box 40220".to_string());
        let resp = response(vec![
            record("mailing_address", "nope", 0, 12, 0.9),
            record("address", "d1", 0, 12, 0.9),
        ]);
        let errs = validate_response(&resp, &sources, &["address"]).expect_err("two failures");
        assert_eq!(errs.len(), 2);
        assert_eq!(
            errs.iter().map(|e| e.record_index).collect::<Vec<_>>(),
            vec![0, 0]
        );
        assert_eq!(
            errs.iter().map(|e| &e.kind).collect::<Vec<_>>(),
            vec![
                &ValidationErrorKind::UnknownTopic,
                &ValidationErrorKind::MissingDoc,
            ]
        );
    }

    #[test]
    fn validator_rejects_short_quote() {
        let mut sources = HashMap::new();
        sources.insert("d1".to_string(), "Mon-Fri 9-5, PO Box 40220".to_string());
        let mut rec = record("hours", "d1", 0, 7, 0.9);
        rec.value = "Mon-Fri".to_string();
        let resp = response(vec![rec]);
        let errs = validate_response(&resp, &sources, &["hours"]).expect_err("short quote");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ValidationErrorKind::ShortQuote);
    }

    #[test]
    fn validator_rejects_value_outside_quote() {
        let mut sources = HashMap::new();
        sources.insert(
            "d1".to_string(),
            "walk-in Tue-Thu only, no mail stop".to_string(),
        );
        let mut rec = record("walkin", "d1", 0, 20, 0.9);
        rec.value = "PO Box 40220".to_string();
        let resp = response(vec![rec]);
        let errs = validate_response(&resp, &sources, &["walkin"]).expect_err("value not in quote");
        assert_eq!(errs.len(), 1);
        assert_eq!(errs[0].kind, ValidationErrorKind::ValueNotInQuote);
    }

    #[test]
    fn normalize_collapses_whitespace_and_case() {
        assert_eq!(normalize("  PO\tBox\n40220 "), "po box 40220");
    }

    #[test]
    fn redact_merges_overlapping_ranges() {
        let out = redact_ignored("abcdefghij", &[(2, 5), (4, 8), (0, 1)]);
        assert_eq!(out, "[...]b[...]ij");
    }

    #[test]
    fn redact_clamps_out_of_bounds_and_unsorted() {
        let out = redact_ignored("abcdef", &[(4, 99), (0, 2)]);
        assert_eq!(out, "[...]cd[...]");
    }
}
