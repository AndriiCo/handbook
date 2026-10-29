//! Managed batch jobs over background Responses.
//!
//! A job is a frozen JSONL dataset plus a state file, so a run
//! survives Ctrl-C: `submit` enqueues, `poll` advances, `collect`
//! gathers. Pacing honors the model's RPM. When upstream documents
//! a native batch-execution endpoint, it slots in behind
//! [`BatchTransport`] without touching the job format.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::model::{BackgroundStatus, ModelPort, ResponsesRequest};

/// One batch item: frozen request + live state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchItem {
    pub key: String,
    pub request: ResponsesRequest,
    pub state: ItemState,
}

/// Item lifecycle.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ItemState {
    Queued,
    Submitted {
        response_id: String,
    },
    Done {
        text: String,
        input_tokens: u32,
        output_tokens: u32,
    },
    Failed {
        error: String,
    },
}

/// A persisted batch job (`job.json` + `requests.jsonl`).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchJob {
    pub id: String,
    pub created_at: i64,
    pub model: String,
    pub items: Vec<BatchItem>,
}

impl BatchJob {
    /// Create + persist a job. Requests are forced to background.
    pub fn create(
        dir: &Path,
        id: &str,
        model: &str,
        now: i64,
        requests: Vec<(String, ResponsesRequest)>,
    ) -> Result<Self> {
        std::fs::create_dir_all(dir).map_err(Error::Io)?;
        let items = requests
            .into_iter()
            .map(|(key, mut request)| {
                request.background = true;
                BatchItem {
                    key,
                    request,
                    state: ItemState::Queued,
                }
            })
            .collect::<Vec<_>>();
        let job = BatchJob {
            id: id.to_string(),
            created_at: now,
            model: model.to_string(),
            items,
        };
        job.save(dir)?;
        // Frozen input dataset (purpose=batch-compatible JSONL).
        let lines = job
            .items
            .iter()
            .map(|item| {
                serde_json::json!({
                    "key": item.key,
                    "model": item.request.model,
                    "instructions": item.request.instructions,
                    "input": item.request.input,
                })
                .to_string()
            })
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(dir.join("requests.jsonl"), lines).map_err(Error::Io)?;
        Ok(job)
    }

    /// Reopen a job for resume (`--batch-poll`).
    pub fn open(dir: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(dir.join("job.json")).map_err(Error::Io)?;
        serde_json::from_str(&text).map_err(|e| Error::Batch(format!("unparseable job: {e}")))
    }

    /// Persist current states.
    pub fn save(&self, dir: &Path) -> Result<()> {
        std::fs::write(
            dir.join("job.json"),
            serde_json::to_string_pretty(self).expect("job serializes"),
        )
        .map_err(Error::Io)?;
        Ok(())
    }

    /// Counts: (queued, submitted, done, failed).
    pub fn counts(&self) -> (usize, usize, usize, usize) {
        let mut counts = (0, 0, 0, 0);
        for item in &self.items {
            match item.state {
                ItemState::Queued => counts.0 += 1,
                ItemState::Submitted { .. } => counts.1 += 1,
                ItemState::Done { .. } => counts.2 += 1,
                ItemState::Failed { .. } => counts.3 += 1,
            }
        }
        counts
    }

    /// Ordered outputs for finished items.
    pub fn collect(&self) -> Vec<(&str, &str)> {
        self.items
            .iter()
            .filter_map(|item| match &item.state {
                ItemState::Done { text, .. } => Some((item.key.as_str(), text.as_str())),
                _ => None,
            })
            .collect()
    }
}

/// Native batch-execution endpoint (reserved — upstream documents
/// only `purpose=batch` uploads today, no run endpoint).
pub trait BatchTransport {
    fn submit_dataset(&self, _file_id: &str) -> Result<String> {
        Err(Error::Batch(
            "native batch execution is unpinned upstream — the harness runs batches over background Responses instead".to_string(),
        ))
    }
}

/// Drives a [`BatchJob`] through a [`ModelPort`] with RPM pacing.
pub struct BatchRunner<'a, P: ModelPort> {
    port: &'a P,
    /// Minimum gap between submissions.
    pub pace: Duration,
    /// Pause between poll rounds.
    pub poll_every: Duration,
}

impl<'a, P: ModelPort> BatchRunner<'a, P> {
    pub fn new(port: &'a P, rpm: u32, poll_every: Duration) -> Self {
        let pace = rpm
            .checked_div(60)
            .filter(|per_sec| *per_sec > 0)
            .map_or(Duration::ZERO, |per_sec| Duration::from_secs(1) / per_sec);
        Self {
            port,
            pace,
            poll_every,
        }
    }

    /// Submit every queued item (paced; checkpoints after each).
    pub fn submit_pending(&self, job: &mut BatchJob, dir: &Path) -> Result<()> {
        for i in 0..job.items.len() {
            if job.items[i].state != ItemState::Queued {
                continue;
            }
            let outcome = self.port.create_response(&job.items[i].request);
            job.items[i].state = match outcome {
                Ok(created) => ItemState::Submitted {
                    response_id: created.id,
                },
                Err(e) => ItemState::Failed {
                    error: e.to_string(),
                },
            };
            job.save(dir)?;
            std::thread::sleep(self.pace);
        }
        Ok(())
    }

    /// One poll round over submitted items. Returns
    /// (done, pending, failed) counts.
    pub fn poll_once(&self, job: &mut BatchJob, dir: &Path) -> Result<(usize, usize, usize)> {
        for i in 0..job.items.len() {
            let ItemState::Submitted { response_id } = &job.items[i].state else {
                continue;
            };
            let response_id = response_id.clone();
            let outcome = self.port.retrieve_response(&response_id);
            match outcome {
                Ok(got) => match got.status {
                    BackgroundStatus::Completed => {
                        let completion = got.completion.unwrap_or_default();
                        job.items[i].state = ItemState::Done {
                            text: completion.text,
                            input_tokens: completion.input_tokens,
                            output_tokens: completion.output_tokens,
                        };
                    }
                    BackgroundStatus::Failed(reason) => {
                        job.items[i].state = ItemState::Failed { error: reason };
                    }
                    BackgroundStatus::Cancelled => {
                        job.items[i].state = ItemState::Failed {
                            error: "cancelled".to_string(),
                        };
                    }
                    BackgroundStatus::Queued | BackgroundStatus::InProgress => {}
                },
                Err(e) => {
                    job.items[i].state = ItemState::Failed {
                        error: e.to_string(),
                    };
                }
            }
            job.save(dir)?;
        }
        let (_, pending, done, failed) = job.counts();
        Ok((done, pending, failed))
    }

    /// Poll until nothing is submitted (or rounds run out).
    pub fn poll_until_done(
        &self,
        job: &mut BatchJob,
        dir: &Path,
        max_rounds: u32,
    ) -> Result<(usize, usize, usize)> {
        for _ in 0..max_rounds {
            let (done, pending, failed) = self.poll_once(job, dir)?;
            if pending == 0 {
                return Ok((done, pending, failed));
            }
            std::thread::sleep(self.poll_every);
        }
        let (_, pending, done, failed) = job.counts();
        Ok((done, pending, failed))
    }
}

/// Batch job directory for one run.
pub fn job_dir(out: &Path, job_id: &str) -> PathBuf {
    out.join("batch").join(job_id)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{InputBlock, StubPort};

    fn req(text: &str) -> ResponsesRequest {
        ResponsesRequest {
            model: "m".to_string(),
            instructions: "i".to_string(),
            input: vec![InputBlock::Text(text.to_string())],
            schema: None,
            max_output_tokens: None,
            reasoning_effort: None,
            background: false,
            previous_response_id: None,
        }
    }

    #[test]
    fn submit_poll_collect_roundtrip() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("job-1");
        let mut job = BatchJob::create(
            &dir,
            "job-1",
            "m",
            1000,
            vec![("a".to_string(), req("A")), ("b".to_string(), req("B"))],
        )
        .unwrap();
        assert!(job.items.iter().all(|i| i.request.background));
        assert_eq!(job.counts(), (2, 0, 0, 0));
        let port = StubPort::new("", "OUT");
        let runner = BatchRunner::new(&port, u32::MAX, Duration::ZERO);
        runner.submit_pending(&mut job, &dir).unwrap();
        assert_eq!(job.counts(), (0, 2, 0, 0));
        // Resume from disk (Ctrl-C survival).
        let mut job = BatchJob::open(&dir).unwrap();
        let (done, pending, failed) = runner.poll_until_done(&mut job, &dir, 3).unwrap();
        assert_eq!((done, pending, failed), (2, 0, 0));
        assert_eq!(job.collect(), vec![("a", "OUT"), ("b", "OUT")]);
    }
}
