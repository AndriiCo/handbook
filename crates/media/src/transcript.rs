//! Transcript annotation + fix model for the Day-3 ingest UI (G3).
//!
//! Staff review machine transcripts turn by turn: flag a turn, leave
//! a note, or fix its text. Every fix rewrites the turn and appends a
//! correction annotation recording before/after, so the audit trail
//! shows what the model heard and what staff confirmed.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::transcribe::Transcript;

/// What an annotation means.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum AnnotationKind {
    /// Staff corrected the turn text (written by [`apply_fix`]).
    Correction,
    /// Turn needs a second look (unclear audio, jargon, PII?).
    Flag,
    /// Free note pinned to the turn.
    Note,
}

/// One annotation on one turn.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Annotation {
    pub turn_index: usize,
    pub kind: AnnotationKind,
    pub note: String,
    pub author: String,
    pub created_at: i64,
}

/// Fix a turn's text and record the correction.
///
/// The turn keeps its index/speaker/timing; only `text` changes. The
/// appended [`Annotation`] quotes before → after.
///
/// # Errors
///
/// Returns [`TranscriptError`] when the turn does not exist, the
/// correction is blank, or the author is blank.
pub fn apply_fix(
    transcript: &mut Transcript,
    annotations: &mut Vec<Annotation>,
    turn_index: usize,
    corrected_text: &str,
    author: &str,
    now_unix: i64,
) -> Result<(), TranscriptError> {
    let Some(turn) = transcript.turns.iter_mut().find(|t| t.index == turn_index) else {
        return Err(TranscriptError::UnknownTurn { turn_index });
    };
    if corrected_text.trim().is_empty() {
        return Err(TranscriptError::EmptyCorrection);
    }
    if author.trim().is_empty() {
        return Err(TranscriptError::EmptyAuthor);
    }
    let before = std::mem::replace(&mut turn.text, corrected_text.trim().to_string());
    annotations.push(Annotation {
        turn_index,
        kind: AnnotationKind::Correction,
        note: format!("{before} → {}", turn.text),
        author: author.trim().to_string(),
        created_at: now_unix,
    });
    Ok(())
}

/// [`apply_fix`] failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TranscriptError {
    UnknownTurn { turn_index: usize },
    EmptyCorrection,
    EmptyAuthor,
}

impl fmt::Display for TranscriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TranscriptError::UnknownTurn { turn_index } => {
                write!(f, "no transcript turn {turn_index}")
            }
            TranscriptError::EmptyCorrection => write!(f, "corrected text is blank"),
            TranscriptError::EmptyAuthor => write!(f, "author is required"),
        }
    }
}

impl std::error::Error for TranscriptError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::TranscriptTurn;

    fn transcript() -> Transcript {
        Transcript {
            turns: vec![
                TranscriptTurn {
                    index: 0,
                    speaker: Some("staff".to_string()),
                    text: "Morning, how can I help?".to_string(),
                    start_ms: 0,
                    end_ms: 2100,
                },
                TranscriptTurn {
                    index: 1,
                    speaker: None,
                    text: "I need an appostille.".to_string(),
                    start_ms: 2100,
                    end_ms: 4000,
                },
            ],
        }
    }

    #[test]
    fn fix_rewrites_and_annotates() {
        let mut transcript = transcript();
        let mut annotations = Vec::new();
        apply_fix(
            &mut transcript,
            &mut annotations,
            1,
            "I need an apostille.",
            "staff-anna",
            1_791_849_600,
        )
        .unwrap();
        assert_eq!(transcript.turns[1].text, "I need an apostille.");
        assert_eq!(transcript.turns[1].start_ms, 2100);
        assert_eq!(annotations.len(), 1);
        assert_eq!(annotations[0].kind, AnnotationKind::Correction);
        assert!(annotations[0].note.contains("appostille"));
        assert!(annotations[0].note.contains("apostille."));
    }

    #[test]
    fn fix_rejects_bad_input() {
        let mut transcript = transcript();
        let mut annotations = Vec::new();
        assert_eq!(
            apply_fix(&mut transcript, &mut annotations, 9, "x", "a", 0).unwrap_err(),
            TranscriptError::UnknownTurn { turn_index: 9 }
        );
        assert_eq!(
            apply_fix(&mut transcript, &mut annotations, 0, "  ", "a", 0).unwrap_err(),
            TranscriptError::EmptyCorrection
        );
        assert_eq!(
            apply_fix(&mut transcript, &mut annotations, 0, "x", "", 0).unwrap_err(),
            TranscriptError::EmptyAuthor
        );
        assert!(annotations.is_empty());
    }
}
