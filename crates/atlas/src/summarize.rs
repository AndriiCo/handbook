//! Map-reduce summarization with a content-addressed cache.
//!
//! Level 0 summarizes each chunk (the map); higher levels merge
//! summaries that fit the budget (the reduce). Every summary is
//! cached by the sha of (instructions + input), so reruns skip
//! unchanged work and `--dry-run` can price the rest.

use std::collections::HashMap;
use std::path::PathBuf;

use crate::chunk::{Chunk, estimate_tokens};
use crate::error::{Error, Result};
use crate::source::sha_hex;

/// One cached summary.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Summary {
    /// Chunk id (`doc#n`) or merge key (`merge:L:…`).
    pub key: String,
    pub text: String,
    pub tokens: u32,
}

/// The model call behind one summary (implemented by the task
/// runner over [`crate::model`]; stubbed in tests).
pub trait Summarizer {
    fn summarize(&self, instructions: &str, input: &str) -> Result<SummaryDraft>;
}

/// Raw model output for one summary call.
#[derive(Debug, Clone)]
pub struct SummaryDraft {
    pub text: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
}

/// File-backed summary cache: `<dir>/<sha>.json`.
pub struct SummaryCache {
    dir: PathBuf,
}

impl SummaryCache {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Cache key: sha over instructions + input + level.
    pub fn key(instructions: &str, input: &str, level: u8) -> String {
        sha_hex(format!("{level}\n{instructions}\n{input}").as_bytes())
    }

    pub fn get(&self, key: &str) -> Option<Summary> {
        let text = std::fs::read_to_string(self.dir.join(format!("{key}.json"))).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn put(&self, key: &str, summary: &Summary) -> Result<()> {
        std::fs::create_dir_all(&self.dir).map_err(Error::Io)?;
        std::fs::write(
            self.dir.join(format!("{key}.json")),
            serde_json::to_string_pretty(summary).expect("summary serializes"),
        )
        .map_err(Error::Io)?;
        Ok(())
    }
}

/// Reduce `chunks` to one brief through `levels` of merging.
/// `budget_tokens` caps each merge input. Returns the final brief
/// plus every intermediate summary (for debug dumps + ledger).
pub fn reduce<S: Summarizer>(
    summarizer: &S,
    cache: &SummaryCache,
    chunks: &[Chunk],
    instructions: &str,
    levels: u8,
    budget_tokens: u32,
) -> Result<ReduceOutcome> {
    let mut all = Vec::new();
    let mut current: Vec<Summary> = Vec::new();
    for chunk in chunks {
        let key = SummaryCache::key(instructions, &chunk.text, 0);
        if let Some(hit) = cache.get(&key) {
            current.push(hit);
            continue;
        }
        let draft = summarizer.summarize(instructions, &chunk.text)?;
        let summary = Summary {
            key: chunk.id.clone(),
            tokens: estimate_tokens(&draft.text),
            text: draft.text,
        };
        cache.put(&key, &summary)?;
        all.push(summary.clone());
        current.push(summary);
    }
    for level in 1..=levels.max(1) {
        if current.len() <= 1 {
            break;
        }
        let mut merged = Vec::new();
        let mut group: Vec<Summary> = Vec::new();
        let mut group_tokens = 0u32;
        for summary in current {
            if !group.is_empty() && group_tokens + summary.tokens > budget_tokens {
                merged.push(merge_group(
                    summarizer,
                    cache,
                    instructions,
                    level,
                    &group,
                    &mut all,
                )?);
                group = Vec::new();
                group_tokens = 0;
            }
            group_tokens += summary.tokens;
            group.push(summary);
        }
        if !group.is_empty() {
            // A lone group at the top level is already the brief.
            if level == levels.max(1) && merged.is_empty() && group.len() == 1 {
                merged = group;
            } else {
                merged.push(merge_group(
                    summarizer,
                    cache,
                    instructions,
                    level,
                    &group,
                    &mut all,
                )?);
            }
        }
        current = merged;
    }
    let brief = current
        .into_iter()
        .map(|s| s.text)
        .collect::<Vec<_>>()
        .join("\n\n");
    Ok(ReduceOutcome { brief, all })
}

fn merge_group<S: Summarizer>(
    summarizer: &S,
    cache: &SummaryCache,
    instructions: &str,
    level: u8,
    group: &[Summary],
    all: &mut Vec<Summary>,
) -> Result<Summary> {
    let input = group
        .iter()
        .map(|s| format!("[{}]\n{}", s.key, s.text))
        .collect::<Vec<_>>()
        .join("\n\n");
    let key = SummaryCache::key(instructions, &input, level);
    if let Some(hit) = cache.get(&key) {
        return Ok(hit);
    }
    let draft = summarizer.summarize(instructions, &input)?;
    let summary = Summary {
        key: format!("merge:{level}:{key}").chars().take(32).collect(),
        tokens: estimate_tokens(&draft.text),
        text: draft.text,
    };
    cache.put(&key, &summary)?;
    all.push(summary.clone());
    Ok(summary)
}

/// Outcome of [`reduce`].
#[derive(Debug)]
pub struct ReduceOutcome {
    pub brief: String,
    /// Every freshly written summary (cache hits excluded).
    pub all: Vec<Summary>,
}

/// Chunk-id → document lookup for citation repair.
pub fn chunk_index(chunks: &[Chunk]) -> HashMap<&str, &Chunk> {
    chunks.iter().map(|c| (c.id.as_str(), c)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    struct Stub {
        calls: Cell<usize>,
    }

    impl Summarizer for Stub {
        fn summarize(&self, _instructions: &str, input: &str) -> Result<SummaryDraft> {
            self.calls.set(self.calls.get() + 1);
            Ok(SummaryDraft {
                text: format!("S{}", input.len()),
                input_tokens: 1,
                output_tokens: 1,
            })
        }
    }

    fn chunks() -> Vec<Chunk> {
        (0..4)
            .map(|i| Chunk {
                id: format!("d#{i}"),
                doc: "d".to_string(),
                text: format!("para {i}"),
                byte_start: 0,
                byte_end: 6,
                tokens: 2,
            })
            .collect()
    }

    #[test]
    fn reduce_merges_and_caches() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = SummaryCache::new(tmp.path().to_path_buf());
        let stub = Stub {
            calls: Cell::new(0),
        };
        let out = reduce(&stub, &cache, &chunks(), "sum", 2, 100).unwrap();
        assert!(!out.brief.is_empty());
        let first_calls = stub.calls.get();
        assert!(first_calls >= 4);
        // Second run hits the cache: zero new calls.
        let out2 = reduce(&stub, &cache, &chunks(), "sum", 2, 100).unwrap();
        assert_eq!(stub.calls.get(), first_calls);
        assert_eq!(out2.brief, out.brief);
        assert!(out2.all.is_empty());
    }

    #[test]
    fn budget_splits_merge_groups() {
        let tmp = tempfile::tempdir().unwrap();
        let cache = SummaryCache::new(tmp.path().to_path_buf());
        let stub = Stub {
            calls: Cell::new(0),
        };
        // Tiny budget forces several level-1 merges.
        let out = reduce(&stub, &cache, &chunks(), "sum", 1, 3).unwrap();
        assert!(!out.brief.is_empty());
        assert!(stub.calls.get() > 4);
    }
}
