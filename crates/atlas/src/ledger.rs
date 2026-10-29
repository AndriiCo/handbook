//! JSONL run ledger: one line per model call.
//!
//! The ledger is the cost + provenance record: model, mode, effort,
//! protocol, input hash, tokens (cached split out), $ estimate,
//! latency, and output path. Appended, never rewritten.

use std::path::PathBuf;

use serde::Serialize;

use crate::config::ModelSpec;
use crate::error::{Error, Result};

/// One recorded model call.
#[derive(Debug, Clone, Serialize)]
pub struct CallRecord {
    pub run_id: String,
    pub at: i64,
    pub task: String,
    pub model: String,
    pub mode: String,
    pub effort: String,
    pub protocol: String,
    pub kind: String,
    pub input_sha: String,
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub cached_tokens: u32,
    pub cost_usd: f64,
    pub elapsed_ms: u64,
    pub output: String,
}

/// Append-only ledger file.
pub struct Ledger {
    path: PathBuf,
}

impl Ledger {
    pub fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub fn path(&self) -> &PathBuf {
        &self.path
    }

    /// Append one record (creates parent dirs on first write).
    pub fn append(&self, record: &CallRecord) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent).map_err(Error::Io)?;
        }
        use std::io::Write;
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .map_err(Error::Io)?;
        writeln!(
            file,
            "{}",
            serde_json::to_string(record).expect("record serializes")
        )
        .map_err(Error::Io)?;
        Ok(())
    }
}

/// $ estimate from the pack's model prices (0 when unpriced).
pub fn estimate_cost(spec: &ModelSpec, input_tokens: u32, output_tokens: u32) -> f64 {
    f64::from(input_tokens) / 1_000_000.0 * spec.input_per_1m
        + f64::from(output_tokens) / 1_000_000.0 * spec.output_per_1m
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_appends_json_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let ledger = Ledger::new(tmp.path().join("ledger.jsonl"));
        let record = CallRecord {
            run_id: "r1".to_string(),
            at: 1000,
            task: "t".to_string(),
            model: "m".to_string(),
            mode: "standard".to_string(),
            effort: "low".to_string(),
            protocol: "fast".to_string(),
            kind: "extract".to_string(),
            input_sha: "ab".to_string(),
            input_tokens: 10,
            output_tokens: 5,
            cached_tokens: 0,
            cost_usd: 0.001,
            elapsed_ms: 12,
            output: "result.json".to_string(),
        };
        ledger.append(&record).unwrap();
        ledger.append(&record).unwrap();
        let text = std::fs::read_to_string(tmp.path().join("ledger.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 2);
        assert!(text.contains("\"run_id\":\"r1\""));
    }
}
