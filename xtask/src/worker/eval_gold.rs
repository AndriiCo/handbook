//! `eval.gold`: the A5 gold-set grading (`xtask eval`) on plant.
//!
//! Payload `{"live": bool, "planted": bool, "prompt": "v1"|"v2"}`.
//! Runs this same binary as `xtask eval --store <job>/store --evals
//! <repo>/evals/authorities [--live] [--planted] [--prompt …]` in a
//! child process (eval exits on its own errors, so a child keeps the
//! worker alive), with `META_API_KEY` from `worker.env` for live
//! drafts. The result is the `quality.json` it writes.

use std::process::Command;

use serde_json::{Value, json};

use super::{JobContext, JobError, JobHandler, JobOutput};

pub struct EvalGold;

/// argv after the binary name (pure: unit-tested).
fn eval_args(ctx: &JobContext, payload: &Value) -> Result<Vec<String>, JobError> {
    let mut args = vec![
        "eval".to_string(),
        "--store".to_string(),
        ctx.dir.join("store").display().to_string(),
        "--evals".to_string(),
        ctx.repo.join("evals/authorities").display().to_string(),
    ];
    if payload["live"].as_bool().unwrap_or(false) {
        args.push("--live".to_string());
    }
    if payload["planted"].as_bool().unwrap_or(false) {
        args.push("--planted".to_string());
    }
    match payload["prompt"].as_str() {
        None => {}
        Some(p @ ("v1" | "v2")) => args.extend(["--prompt".to_string(), p.to_string()]),
        Some(other) => {
            return Err(JobError::fatal(format!(
                "prompt must be v1 or v2, not {other:?}"
            )));
        }
    }
    Ok(args)
}

impl JobHandler for EvalGold {
    fn kind(&self) -> &'static str {
        "eval.gold"
    }

    fn run(&self, ctx: &JobContext, payload: &Value) -> Result<JobOutput, JobError> {
        let args = eval_args(ctx, payload)?;
        let live = args.iter().any(|a| a == "--live");
        let exe =
            std::env::current_exe().map_err(|e| JobError::fatal(format!("current_exe: {e}")))?;
        let mut cmd = Command::new(exe);
        cmd.args(&args).current_dir(&ctx.repo);
        if live {
            let Some(key) = ctx.env.get("META_API_KEY") else {
                return Err(JobError::fatal(
                    "live eval needs META_API_KEY in worker.env",
                ));
            };
            cmd.env("META_API_KEY", key);
        }
        let out = cmd
            .output()
            .map_err(|e| JobError::retry(format!("run xtask eval: {e}")))?;
        let _ = std::fs::write(ctx.dir.join("eval.stdout"), &out.stdout);
        let _ = std::fs::write(ctx.dir.join("eval.stderr"), &out.stderr);
        if !out.status.success() {
            let tail = String::from_utf8_lossy(&out.stderr);
            let tail: String = tail.lines().rev().take(3).collect::<Vec<_>>().join(" | ");
            return Err(JobError::retry(format!(
                "xtask eval exited {}: {tail}",
                out.status
            )));
        }
        let quality = std::fs::read(ctx.dir.join("store/quality.json"))
            .map_err(|e| JobError::fatal(format!("eval wrote no quality.json: {e}")))?;
        let value: Value = serde_json::from_slice(&quality)
            .map_err(|e| JobError::fatal(format!("quality.json: {e}")))?;
        let summary = json!({
            "gold_count": value["gold_count"],
            "quote_found_rate": value["quote_found_rate"],
            "topic_accuracy": value["topic_accuracy"],
            "model": value["model"],
        });
        Ok(JobOutput::json(&value, summary))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> JobContext {
        JobContext {
            id: 1,
            dir: "/w/job-1-a1".into(),
            repo: "/repo".into(),
            cache_dir: "/w".into(),
            env: super::super::WorkerEnv::default(),
        }
    }

    #[test]
    fn args_follow_the_payload() {
        assert_eq!(
            eval_args(&ctx(), &json!({})).unwrap(),
            [
                "eval",
                "--store",
                "/w/job-1-a1/store",
                "--evals",
                "/repo/evals/authorities"
            ]
        );
        let args = eval_args(
            &ctx(),
            &json!({"live": true, "planted": true, "prompt": "v2"}),
        )
        .unwrap();
        assert_eq!(&args[5..], ["--live", "--planted", "--prompt", "v2"]);
        assert!(eval_args(&ctx(), &json!({"prompt": "v9"})).is_err());
    }

    #[test]
    fn live_eval_without_a_key_fails_before_spawning() {
        let err = EvalGold.run(&ctx(), &json!({"live": true})).unwrap_err();
        if std::env::var("META_API_KEY").is_err() {
            assert!(err.message.contains("META_API_KEY"), "{}", err.message);
            assert!(!err.retryable);
        }
    }
}
