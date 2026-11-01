//! `plant-run`: drive long inference jobs on plant over drop-safe ssh.
//!
//! Every remote step runs inside a named tmux session on plant, so a
//! lost (mobile/satellite) ssh connection never kills the job. All
//! xtask invocations below are short-lived: they send keys, capture
//! panes, or copy logs, then exit. Reconnect any time with `attach`,
//! which replaces itself with an interactive
//! `ssh -t plant 'tmux attach ...'`.
//!
//! ```sh
//! cargo xtask plant-run setup [--host plant]   # deploy scripts + sample image
//! cargo xtask plant-run models [--model-url U --mmproj-url U]  # weights via pasted token
//! cargo xtask plant-run start --job job-001 --image syn-doc-clean.png
//! cargo xtask plant-run status --job job-001
//! cargo xtask plant-run log --job job-001 [--tail 50]
//! cargo xtask plant-run fetch --job job-001 [--out <dir>]
//! cargo xtask plant-run attach --job job-001
//! ```
//!
//! Payload sources live in `ops/plant-inference/` (run-job.sh,
//! MODEL.conf, prompt-v1.txt); `setup` copies them to `~/models`
//! with `run-job.sh` executable, and ships a generated synthetic
//! sample image. Model weights (`.gguf`) are a manual download
//! (Hugging Face auth); `start` names whatever is missing instead
//! of failing cryptically.
//!
//! Console output persists on plant at `~/models/<job>.console.log`
//! (tmux `pipe-pane`), so `log` works even after the session ends.
//! `fetch` copies verdict + metrics + logs to
//! `web/shots/inference/<job>/` (git-ignored), where later analysis
//! reads them without pasting anything into chat.

use std::path::PathBuf;
use std::process::Command;

/// tmux sessions are named `infer-<job>`.
const SESSION_PREFIX: &str = "infer-";
/// Local landing dir for `fetch` (git-ignored).
const DEFAULT_OUT: &str = "web/shots/inference";
/// Files `fetch` copies when present.
const JOB_FILES: [&str; 5] = [
    "verdict.txt",
    "metrics.json",
    "stderr.txt",
    "console.log",
    "vram.log",
];

/// Payload files deployed by `setup` (repo-relative sources).
const PAYLOAD_DIR: &str = "ops/plant-inference";
const PAYLOAD_FILES: [&str; 3] = ["run-job.sh", "MODEL.conf", "prompt-v1.txt"];
/// Default weight URLs (override with --model-url / --mmproj-url;
/// filenames verified on plant, not from here).
const MODEL_URL: &str = "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct-GGUF/resolve/main/qwen2.5-vl-7b-instruct-q4_k_m.gguf?download=true";
const MMPROJ_URL: &str = "https://huggingface.co/Qwen/Qwen2.5-VL-7B-Instruct-GGUF/resolve/main/mmproj-qwen2.5-vl-7b-instruct-q8_0.gguf?download=true";

/// `plant-run <action> --job <id> [--image <png>] [--host <ssh>]
/// [--tail <n>] [--out <dir>]` (`setup` takes no --job).
pub fn run(args: &[String]) {
    let action = args.first().map(String::as_str).unwrap_or("");
    let host = crate::flag(args, "host").unwrap_or_else(|| "plant".to_string());
    if action == "setup" {
        cmd_setup(&host);
        return;
    }
    if action == "models" {
        let model_url = crate::flag(args, "model-url").unwrap_or_else(|| MODEL_URL.to_string());
        let mmproj_url = crate::flag(args, "mmproj-url").unwrap_or_else(|| MMPROJ_URL.to_string());
        cmd_models(&host, &model_url, &mmproj_url);
        return;
    }
    let job = clean_id(&crate::required(args, "job"), "job");
    match action {
        "start" => {
            let image = clean_name(&crate::required(args, "image"), "image");
            cmd_start(&host, &job, &image);
        }
        "status" => cmd_status(&host, &job),
        "log" => {
            let tail = crate::flag(args, "tail").unwrap_or_else(|| "50".to_string());
            cmd_log(&host, &job, &tail);
        }
        "fetch" => {
            let out = crate::flag(args, "out")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(DEFAULT_OUT).join(&job));
            cmd_fetch(&host, &job, &out);
        }
        "attach" => cmd_attach(&host, &job),
        _ => fail("want one of: setup|models|start|status|log|fetch|attach (--job <id>)"),
    }
}

/// Shell-safe job id: short, no slashes, no spaces.
fn clean_id(raw: &str, what: &str) -> String {
    let ok = !raw.is_empty()
        && raw.len() <= 40
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if !ok {
        fail(&format!("bad --{what} (want 1-40 of A-Za-z0-9_-)"));
    }
    raw.to_string()
}

/// Shell-safe file name inside ~/models: no slashes, no spaces.
fn clean_name(raw: &str, what: &str) -> String {
    let ok = !raw.is_empty()
        && raw.len() <= 80
        && raw
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if !ok {
        fail(&format!("bad --{what} (file name in ~/models, no paths)"));
    }
    raw.to_string()
}

fn fail(message: &str) -> ! {
    eprintln!("plant-run: {message}");
    std::process::exit(1);
}

/// Quote one word for POSIX sh (single quotes + `'\''` escapes).
fn shell_quote(word: &str) -> String {
    format!("'{}'", word.replace('\'', "'\\''"))
}

/// Run one remote `sh -lc` script over fail-fast ssh; return stdout.
///
/// ssh joins its remote arguments with spaces, so the whole remote
/// command goes as ONE argument with the script shell-quoted.
/// Passing `sh`, `-lc`, script as separate args silently ran bare
/// `chmod` (single-line scripts) while multi-line ones survived by
/// luck of embedded newlines.
fn ssh(host: &str, script: &str) -> String {
    let remote = format!("sh -lc {}", shell_quote(script));
    let out = Command::new("ssh")
        .args([
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=15",
            host,
            "--",
            &remote,
        ])
        .output()
        .unwrap_or_else(|e| fail(&format!("spawn ssh: {e}")));
    if !out.status.success() {
        let stdout = String::from_utf8_lossy(&out.stdout);
        let stderr = String::from_utf8_lossy(&out.stderr);
        // Remote scripts report problems on stdout (MISSING:/HAS_SESSION);
        // prefer that over ssh's own stderr.
        let detail = stdout
            .lines()
            .last()
            .filter(|line| !line.is_empty())
            .or_else(|| stderr.lines().last())
            .unwrap_or("ssh failed");
        fail(&format!("ssh exited {}: {detail}", out.status));
    }
    String::from_utf8_lossy(&out.stdout).to_string()
}

fn session(job: &str) -> String {
    format!("{SESSION_PREFIX}{job}")
}

/// Copy one local file to ~/models on plant.
fn scp_to(host: &str, local: &std::path::Path, remote_name: &str) {
    let status = Command::new("scp")
        .args([
            "-q",
            "-o",
            "BatchMode=yes",
            "-o",
            "ConnectTimeout=15",
            &local.to_string_lossy().into_owned(),
            &format!("{host}:~/models/{remote_name}"),
        ])
        .status()
        .unwrap_or_else(|e| fail(&format!("spawn scp: {e}")));
    if !status.success() {
        fail(&format!("scp {} -> plant failed", local.display()));
    }
}

fn cmd_setup(host: &str) {
    for file in PAYLOAD_FILES {
        let local = PathBuf::from(PAYLOAD_DIR).join(file);
        if !local.is_file() {
            fail(&format!("payload missing in repo: {}", local.display()));
        }
        scp_to(host, &local, file);
    }
    ssh(host, "chmod +x $HOME/models/run-job.sh && echo SETUP_OK");
    // Ship a generated synthetic sample image (stdlib-only generator).
    let tmp = std::env::temp_dir().join("plant-inference-samples");
    std::fs::create_dir_all(&tmp).unwrap_or_else(|e| fail(&format!("mkdir tmp: {e}")));
    let img_gen = Command::new("python3")
        .arg("web/scripts/gen-demo-samples.py")
        .arg(&tmp)
        .status()
        .unwrap_or_else(|e| fail(&format!("spawn python3: {e}")));
    if img_gen.success() {
        scp_to(host, &tmp.join("syn-doc-clean.png"), "syn-doc-clean.png");
        println!("plant-run: setup complete (scripts + syn-doc-clean.png)");
    } else {
        println!(
            "plant-run: scripts deployed; sample image NOT generated (run web/scripts/gen-demo-samples.py by hand)"
        );
    }
}

/// Read a line with terminal echo disabled (token never in history).
/// Restores echo on every path; warns (not fails) without a tty.
fn read_secret(prompt: &str) -> String {
    use std::io::Write as _;
    print!("{prompt}");
    std::io::stdout().flush().ok();
    let tty = Command::new("stty")
        .arg("-echo")
        .status()
        .map(|s| s.success());
    let mut line = String::new();
    let read = std::io::stdin().read_line(&mut line);
    let _ = Command::new("stty").arg("echo").status();
    println!();
    if !matches!(tty, Ok(true)) {
        eprintln!("plant-run: no tty for hidden input; consider piping carefully");
    }
    if read.is_err() || line.trim().is_empty() {
        fail("empty token (gated repos also need the license accepted in a browser)");
    }
    line.trim().to_string()
}

#[cfg(unix)]
fn restrict(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt as _;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).ok();
}

#[cfg(not(unix))]
fn restrict(_path: &std::path::Path) {}

/// Download both weight files with a pasted HF token, then scp them
/// to plant. The token lives only in process memory plus one 0600
/// curl config file, deleted afterwards; it never appears in argv,
/// history, logs, or the repo.
fn cmd_models(host: &str, model_url: &str, mmproj_url: &str) {
    println!("plant-run: paste a read-only Hugging Face token (echo off).");
    println!("plant-run: gated repos also need the license accepted at huggingface.co first.");
    let token = read_secret("HF token: ");
    let tmp = std::env::temp_dir().join("plant-inference-weights");
    std::fs::create_dir_all(&tmp).unwrap_or_else(|e| fail(&format!("mkdir tmp: {e}")));
    let cfg = tmp.join("curl.conf");
    std::fs::write(
        &cfg,
        format!("header = \"Authorization: Bearer {token}\"\n"),
    )
    .unwrap_or_else(|e| fail(&format!("write curl conf: {e}")));
    restrict(&cfg);
    let jobs = [("model.gguf", model_url), ("mmproj.gguf", mmproj_url)];
    for (name, url) in jobs {
        let out = tmp.join(name);
        let status = Command::new("curl")
            .args(["-fSL", "-K"])
            .arg(&cfg)
            .args(["-o"])
            .arg(&out)
            .arg(url)
            .status()
            .unwrap_or_else(|e| fail(&format!("spawn curl: {e}")));
        if !status.success() {
            let _ = std::fs::remove_file(&cfg);
            fail(&format!(
                "download {name} failed (wrong URL? license not accepted?)"
            ));
        }
        let bytes = out.metadata().map(|m| m.len()).unwrap_or(0);
        println!("plant-run: {name} {bytes} bytes");
        scp_to(host, &out, name);
        let _ = std::fs::remove_file(&out);
    }
    let _ = std::fs::remove_file(&cfg);
    println!("plant-run: weights on plant (token wiped from disk)");
}

fn cmd_start(host: &str, job: &str, image: &str) {
    let session = session(job);
    let script = format!(
        "MISSING=\"\"\n\
         command -v tmux >/dev/null || MISSING=\"$MISSING tmux\" \n\
         test -x $HOME/models/run-job.sh || MISSING=\"$MISSING run-job.sh[cargo xtask plant-run setup]\" \n\
         test -f $HOME/models/MODEL.conf || MISSING=\"$MISSING MODEL.conf[setup]\" \n\
         test -f $HOME/models/prompt-v1.txt || MISSING=\"$MISSING prompt-v1.txt[setup]\" \n\
         test -f $HOME/models/{image} || MISSING=\"$MISSING {image}[setup ships syn-doc-clean.png]\" \n\
         if test -f $HOME/models/MODEL.conf; then\n\
           . $HOME/models/MODEL.conf\n\
           test -f $HOME/models/$MODEL_FILE || MISSING=\"$MISSING $MODEL_FILE[cargo xtask plant-run models]\"\n\
           test -f $HOME/models/$MMPROJ_FILE || MISSING=\"$MISSING $MMPROJ_FILE[cargo xtask plant-run models]\"\n\
         fi\n\
         test -n \"$MISSING\" && {{ echo \"MISSING:$MISSING\"; exit 6; }}\n\
         tmux has-session -t {session} 2>/dev/null && {{ echo HAS_SESSION; exit 4; }}\n\
         tmux new-session -d -s {session} -x 200 -y 60\n\
         tmux pipe-pane -t {session} -o 'cat >> $HOME/models/{job}.console.log'\n\
         tmux send-keys -t {session} './run-job.sh {job} {image}' Enter\n\
         echo STARTED"
    );
    let out = ssh(host, &script);
    if out.contains("STARTED") {
        println!("plant-run: {job} started in tmux {session} (safe to disconnect)");
    } else {
        fail(&format!("unexpected start output: {out}"));
    }
}

fn cmd_status(host: &str, job: &str) {
    let session = session(job);
    let script = format!(
        "if tmux has-session -t {session} 2>/dev/null; then\n\
           echo STATE=RUNNING\n\
           tmux capture-pane -p -t {session} | tail -5\n\
         elif test -f $HOME/models/{job}.metrics.json; then\n\
           echo STATE=DONE\n\
           cat $HOME/models/{job}.metrics.json\n\
         else\n\
           echo STATE=UNKNOWN\n\
         fi"
    );
    print!("{}", ssh(host, &script));
}

fn cmd_log(host: &str, job: &str, tail: &str) {
    let tail_n: u32 = tail
        .parse()
        .unwrap_or_else(|_| fail("bad --tail (positive integer)"));
    if tail_n == 0 || tail_n > 1000 {
        fail("bad --tail (1..=1000)");
    }
    let session = session(job);
    let script = format!(
        "if test -f $HOME/models/{job}.console.log; then\n\
           tail -n {tail_n} $HOME/models/{job}.console.log\n\
         elif tmux has-session -t {session} 2>/dev/null; then\n\
           tmux capture-pane -p -t {session} | tail -n {tail_n}\n\
         else\n\
           echo NO_LOG; exit 7\n\
         fi"
    );
    print!("{}", ssh(host, &script));
}

fn cmd_fetch(host: &str, job: &str, out: &std::path::Path) {
    std::fs::create_dir_all(out).unwrap_or_else(|e| fail(&format!("mkdir {}: {e}", out.display())));
    let mut missing = 0;
    for file in JOB_FILES {
        let remote = format!("~/models/{job}.{file}");
        let status = Command::new("scp")
            .args([
                "-q",
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=15",
                &format!("{host}:{remote}"),
                &out.join(format!("{job}.{file}"))
                    .to_string_lossy()
                    .into_owned(),
            ])
            .status()
            .unwrap_or_else(|e| fail(&format!("spawn scp: {e}")));
        if !status.success() {
            eprintln!("plant-run: {job}.{file} not on plant yet (job still running?)");
            missing += 1;
        }
    }
    println!(
        "plant-run: {job} fetched to {} ({missing} missing)",
        out.display()
    );
}

/// Replace this process with an interactive tmux attach: the human
/// (or a later agent turn) lands inside the live console.
fn cmd_attach(host: &str, job: &str) {
    let session = session(job);
    let status = Command::new("ssh")
        .args(["-t", host, &format!("tmux attach -t {session}")])
        .status()
        .unwrap_or_else(|e| fail(&format!("spawn ssh: {e}")));
    if !status.success() {
        fail("attach ended (no such session, or link dropped — retry)");
    }
}
