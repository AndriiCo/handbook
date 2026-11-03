//! `precheck.glimmer`: dispatch stub for the self-hosted Muse Glimmer
//! precheck on the plant GPU (builder fills in the run).
//!
//! Already wired: the kind name (matches the server's `KINDS`), the
//! GPU flag (the worker holds `~/apostille-pipeline/gpu.lock` for the
//! whole run), and the job dir. Until [`PrecheckGlimmer::run`] does
//! real work, [`JobHandler::available`] keeps the kind out of `hello`,
//! so queued Glimmer jobs wait `pending` instead of failing.
//!
//! To finish: load the model from `<cache_dir>/models/` (operator
//! decision: `/srv/scratch/apostille/models`), run it on the payload,
//! return `JobOutput::json(...)`, and flip `available` to true.

use serde_json::Value;

use super::{JobContext, JobError, JobHandler, JobOutput};

pub struct PrecheckGlimmer;

impl JobHandler for PrecheckGlimmer {
    fn kind(&self) -> &'static str {
        "precheck.glimmer"
    }

    fn gpu(&self) -> bool {
        true
    }

    fn available(&self) -> bool {
        false
    }

    fn run(&self, _ctx: &JobContext, _payload: &Value) -> Result<JobOutput, JobError> {
        Err(JobError::fatal(
            "precheck.glimmer has no runner on this worker yet \
             (xtask/src/worker/precheck_glimmer.rs)",
        ))
    }
}
