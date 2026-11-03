//! Daily live-call ledger behind `meta.daily_call_cap`.
//!
//! One JSON line per live Meta HTTP attempt (retries included) in
//! `<meta-cache>/ledger/<YYYY-MM-DD>.jsonl` (UTC day). Every process
//! that shares a cache dir (web, xtask, atlas, workers, the A2 Python
//! recorder) shares the count. Cache hits never write a line.
//!
//! Lines are appended with one `O_APPEND` write each, so concurrent
//! writers never interleave. The check-then-call window is not locked:
//! with N calls in flight the day can overshoot the cap by at most N.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::{CacheError, ensure_dir, utc_date};

/// Default `meta.daily_call_cap` (every pipeline, per UTC day). Keep
/// in sync with `web::telemetry::DEFAULT_DAILY_CALL_CAP`.
pub const DEFAULT_DAILY_CALL_CAP: u64 = 400;

/// Daily cap over a ledger directory.
#[derive(Debug, Clone)]
pub struct Budget {
    dir: PathBuf,
    cap: u64,
}

impl Budget {
    /// Ledger under `dir` (normally `<meta-cache>/ledger`).
    pub fn new(dir: impl Into<PathBuf>, cap: u64) -> Self {
        Self {
            dir: dir.into(),
            cap,
        }
    }

    pub fn cap(&self) -> u64 {
        self.cap
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn day_file(&self, now: i64) -> PathBuf {
        self.dir.join(format!("{}.jsonl", utc_date(now)))
    }

    /// Live calls recorded so far on `now`'s UTC day.
    pub fn used(&self, now: i64) -> u64 {
        std::fs::read(self.day_file(now))
            .map(|bytes| bytes.iter().filter(|b| **b == b'\n').count() as u64)
            .unwrap_or(0)
    }

    /// `Err(OverBudget)` once today's live calls reached the cap.
    pub fn check(&self, now: i64) -> Result<(), CacheError> {
        let used = self.used(now);
        if used >= self.cap {
            return Err(CacheError::OverBudget {
                cap: self.cap,
                used,
            });
        }
        Ok(())
    }

    /// Count one live attempt. Storage errors are returned but callers
    /// may ignore them: the answer already arrived and is paid for.
    pub fn spend(&self, now: i64, path: &str, model: &str, status: u16) -> Result<(), CacheError> {
        ensure_dir(&self.dir)?;
        let line = serde_json::json!({
            "at": now, "path": path, "model": model, "status": status,
            "pid": std::process::id(),
        })
        .to_string();
        let file = self.day_file(now);
        std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&file)
            .and_then(|mut f| f.write_all(format!("{line}\n").as_bytes()))
            .map_err(|e| CacheError::Io(format!("ledger {}: {e}", file.display())))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cap_counts_today_only_and_zero_stops_everything() {
        let dir = tempfile::tempdir().unwrap();
        let budget = Budget::new(dir.path().join("ledger"), 2);
        let day = 1_793_664_000; // 2026-11-03T00:00Z
        assert_eq!(budget.used(day), 0);
        budget.check(day).unwrap();
        budget
            .spend(day + 5, "/v1/chat/completions", "m", 200)
            .unwrap();
        budget
            .spend(day + 9, "/v1/chat/completions", "m", 429)
            .unwrap();
        assert_eq!(budget.used(day), 2);
        assert_eq!(
            budget.check(day + 10),
            Err(CacheError::OverBudget { cap: 2, used: 2 })
        );
        // The next UTC day starts empty.
        budget.check(day + 86_400).unwrap();
        assert!(
            Budget::new(dir.path().join("ledger"), 0)
                .check(day + 86_400)
                .is_err()
        );
    }
}
