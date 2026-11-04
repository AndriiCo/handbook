//! `eval.gold`: the A5 gold-set grading (`xtask eval`) on plant.
//!
//! Payload `{"live": bool, "local": bool, "base_url": "…", "model": "…",
//! "planted": bool, "prompt": "v1"|"v2", "gold": "<dir under evals/>"}`
//! (`gold` defaults to `authorities`; `authorities-real` holds cases
//! from `xtask gold-export`).
//! Runs this same binary as `xtask eval --store <job>/store --evals
//! <repo>/evals/<gold> [--live | --local] [--planted] [--prompt …]` in a
//! child process (eval exits on its own errors, so a child keeps the
//! worker alive), with `META_API_KEY` set in the child's environment
//! from `welcome.settings.meta_api_key` for live drafts. `local` grades
//! an OpenAI-compatible endpoint serving a Meta model (llama.cpp
//! `llama-server` on plant): the payload's `base_url` / `model`, else
//! the `local_model_url` / `local_model` settings, passed to the child
//! as `LOCAL_MODEL_URL` / `LOCAL_MODEL` (and `local_model_key` as
//! `LOCAL_MODEL_KEY`). The child never inherits those variables from
//! the worker's own environment. The result is
//! the `quality.json` it writes; compare its `quote_found_rate` and
//! `value_in_quote_rate` with a Spark run before a local model drafts.

use std::process::Command;

use serde_json::{Value, json};

use super::{JobContext, JobError, JobHandler, JobOutput, keys};

pub struct EvalGold;

/// argv after the binary name (pure: unit-tested).
fn eval_args(ctx: &JobContext, payload: &Value) -> Result<Vec<String>, JobError> {
    let gold = payload["gold"].as_str().unwrap_or("authorities");
    let named = (1..=40).contains(&gold.len())
        && gold
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
    if !named {
        return Err(JobError::fatal(format!(
            "gold must name a directory under evals/ ([a-z0-9-]), not {gold:?}"
        )));
    }
    let mut args = vec![
        "eval".to_string(),
        "--store".to_string(),
        ctx.dir.join("store").display().to_string(),
        "--evals".to_string(),
        ctx.repo.join("evals").join(gold).display().to_string(),
    ];
    let live = payload["live"].as_bool().unwrap_or(false);
    let local = payload["local"].as_bool().unwrap_or(false);
    if live && local {
        return Err(JobError::fatal("live and local are exclusive"));
    }
    if live {
        args.push("--live".to_string());
    }
    if local {
        args.push("--local".to_string());
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

/// The child's `LOCAL_MODEL_*` environment for a `local` run: payload
/// `base_url` / `model` over the settings (pure: unit-tested). The
/// model must name a Meta family; the key comes from the settings only.
fn local_env(ctx: &JobContext, payload: &Value) -> Result<Vec<(&'static str, String)>, JobError> {
    let [url_var, model_var, key_var] = crate::eval::LOCAL_ENV;
    let pick = |field: &str, key: &str| {
        payload[field]
            .as_str()
            .map(str::to_string)
            .or_else(|| ctx.settings.get(key))
            .unwrap_or_default()
    };
    let base = pick("base_url", keys::LOCAL_MODEL_URL);
    if !(base.starts_with("http://") || base.starts_with("https://")) {
        return Err(JobError::fatal(format!(
            "local eval needs an http(s) base_url in the payload (or the {} setting)",
            keys::LOCAL_MODEL_URL
        )));
    }
    let model = pick("model", keys::LOCAL_MODEL);
    if !crate::eval::is_meta_model(&model) {
        return Err(JobError::fatal(format!(
            "local eval needs a Meta model (muse-… or Llama) in model (or the {} setting), \
             not {model:?}",
            keys::LOCAL_MODEL
        )));
    }
    let mut env = vec![(url_var, base), (model_var, model)];
    if let Some(key) = ctx.settings.get(keys::LOCAL_MODEL_KEY) {
        env.push((key_var, key));
    }
    Ok(env)
}

impl JobHandler for EvalGold {
    fn kind(&self) -> &'static str {
        "eval.gold"
    }

    fn run(&self, ctx: &JobContext, payload: &Value) -> Result<JobOutput, JobError> {
        let args = eval_args(ctx, payload)?;
        let live = args.iter().any(|a| a == "--live");
        let local = if args.iter().any(|a| a == "--local") {
            local_env(ctx, payload)?
        } else {
            Vec::new()
        };
        let exe =
            std::env::current_exe().map_err(|e| JobError::fatal(format!("current_exe: {e}")))?;
        let mut cmd = Command::new(exe);
        cmd.args(&args).current_dir(&ctx.repo);
        // Only the server's values reach the child, never the worker's
        // own environment.
        cmd.env_remove("META_API_KEY");
        for var in crate::eval::LOCAL_ENV {
            cmd.env_remove(var);
        }
        if live {
            let Some(key) = ctx.settings.get(keys::META_API_KEY) else {
                return Err(JobError::fatal(
                    "live eval needs the Meta key: set meta.api_key in /config",
                ));
            };
            cmd.env("META_API_KEY", key);
        }
        cmd.envs(local);
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
            "value_in_quote_rate": value["value_in_quote_rate"],
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
            settings: super::super::Settings::default(),
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
        let real = eval_args(&ctx(), &json!({"gold": "authorities-real"})).unwrap();
        assert_eq!(real[4], "/repo/evals/authorities-real");
        for bad in ["../secrets", "", "Authorities", "a/b"] {
            assert!(eval_args(&ctx(), &json!({"gold": bad})).is_err(), "{bad:?}");
        }
    }

    /// D-4: `local` adds `--local`; its endpoint comes from the payload
    /// over the settings, and only a Meta model id passes.
    #[test]
    fn local_eval_takes_its_endpoint_from_payload_or_settings() {
        let args = eval_args(&ctx(), &json!({"local": true})).unwrap();
        assert_eq!(&args[5..], ["--local"]);
        assert!(eval_args(&ctx(), &json!({"local": true, "live": true})).is_err());
        let mut with_env = ctx();
        with_env.settings = super::super::Settings::of(&[
            (keys::LOCAL_MODEL_URL, "http://127.0.0.1:8080/v1"),
            (keys::LOCAL_MODEL, "llama-local"),
            (keys::LOCAL_MODEL_KEY, "k"),
        ]);
        let env = local_env(&with_env, &json!({"local": true})).unwrap();
        assert_eq!(
            env,
            [
                ("LOCAL_MODEL_URL", "http://127.0.0.1:8080/v1".to_string()),
                ("LOCAL_MODEL", "llama-local".to_string()),
                ("LOCAL_MODEL_KEY", "k".to_string()),
            ]
        );
        let over = local_env(
            &with_env,
            &json!({"local": true, "base_url": "http://plant:8081/v1", "model": "Llama-3.1-8B"}),
        )
        .unwrap();
        assert_eq!(over[0].1, "http://plant:8081/v1");
        assert_eq!(over[1].1, "Llama-3.1-8B");
        for bad in [
            json!({"model": "acme-7b"}),
            json!({"base_url": "file:///x"}),
        ] {
            let err = local_env(&with_env, &bad).unwrap_err();
            assert!(!err.retryable, "{bad}");
        }
        // No endpoint in the payload or settings: refused before
        // spawning, whatever LOCAL_MODEL_URL the process environment holds.
        let err = EvalGold.run(&ctx(), &json!({"local": true})).unwrap_err();
        assert!(err.message.contains("base_url"), "{}", err.message);
    }

    /// No `meta_api_key` setting: refused, even when the worker's own
    /// environment (or a `worker.env`) has `META_API_KEY`.
    #[test]
    fn live_eval_without_a_key_fails_before_spawning() {
        let err = EvalGold.run(&ctx(), &json!({"live": true})).unwrap_err();
        assert!(err.message.contains("meta.api_key"), "{}", err.message);
        assert!(!err.retryable);
    }
}
