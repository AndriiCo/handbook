//! Staff "Load packs" (P397): load the live release's jurisdiction
//! packs into the store from the admin UI, as the service user.
//!
//! Replaces the root-run `ops-packs-seed.sh` step for every deploy:
//! the handler runs the release's own `xtask packs-load` (validate
//! first, refuse on any error, authority/domain/source/task rows only
//! — never facts or captures; re-runs are no-ops) against the live
//! store. One run at a time; every run is audited with before/after
//! authority counts. Mounted with the other mission admin routers, so
//! the admin IP + host + session + CSRF guards apply.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::{
    Json, Router,
    extract::State,
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
};
use handbook_store::{db, store::Store};
use serde::Deserialize;

use crate::{AppState, admin::AdminAuth, unix_now};

/// The loadable sets, in load order: curated top-level packs first so
/// world rows that share a curated slug merge into the curated row.
const SETS: &[(&str, &str)] = &[
    ("curated", ""),
    ("world", "world"),
    ("world-nonparty", "world-nonparty"),
];

static RUNNING: AtomicBool = AtomicBool::new(false);

pub fn admin_router(state: AppState) -> Router {
    Router::<AppState>::new()
        .route("/apostille/api/v1/admin/packs", get(list_sets))
        .route("/apostille/api/v1/admin/packs/load", post(load_set))
        .with_state(state)
}

/// Where the packs and the loader live: `(packs root, xtask binary)`.
///
/// The loader is the `xtask` beside the running `web` binary (a
/// production release dir `/srv/apostille/releases/<v>/` ships both;
/// a staging checkout builds both into `target/release/`). The packs
/// root is `PACKS_DIR` when set, else `packs/` beside the binary
/// (production), else `packs/` under the working directory (a staging
/// checkout runs from its root).
fn release_paths() -> Option<(PathBuf, PathBuf)> {
    let exe_dir = std::env::current_exe()
        .ok()?
        .canonicalize()
        .ok()?
        .parent()
        .map(Path::to_path_buf)?;
    let xtask = exe_dir.join("xtask");
    if let Some(dir) = std::env::var_os("PACKS_DIR") {
        return Some((PathBuf::from(dir), xtask));
    }
    let beside = exe_dir.join("packs");
    if beside.is_dir() {
        return Some((beside, xtask));
    }
    let cwd = std::env::current_dir().ok()?.join("packs");
    cwd.is_dir().then_some((cwd, xtask))
}

fn set_dir(packs: &Path, sub: &str) -> PathBuf {
    if sub.is_empty() {
        packs.to_path_buf()
    } else {
        packs.join(sub)
    }
}

fn pack_files(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "json"))
                .count()
        })
        .unwrap_or(0)
}

fn authority_count(state: &AppState) -> Option<i64> {
    let conn = db::open(&Store::new(&state.store_root)).ok()?;
    conn.query_row("SELECT COUNT(*) FROM authority WHERE id <> 0", [], |row| {
        row.get(0)
    })
    .ok()
}

fn err(status: StatusCode, message: impl Into<String>) -> axum::response::Response {
    (status, Json(serde_json::json!({ "error": message.into() }))).into_response()
}

async fn list_sets(State(state): State<AppState>, _auth: AdminAuth) -> impl IntoResponse {
    let release = release_paths().map(|(packs, _)| packs);
    let sets: Vec<_> = SETS
        .iter()
        .map(|(name, sub)| {
            let packs = release
                .as_deref()
                .map(|r| pack_files(&set_dir(r, sub)))
                .unwrap_or(0);
            serde_json::json!({ "set": name, "packs": packs })
        })
        .collect();
    Json(serde_json::json!({
        "sets": sets,
        "authorities": authority_count(&state),
        "running": RUNNING.load(Ordering::SeqCst),
    }))
}

#[derive(Debug, Deserialize)]
struct LoadBody {
    set: String,
}

/// Clears [`RUNNING`] however the handler exits.
struct RunGuard;

impl Drop for RunGuard {
    fn drop(&mut self) {
        RUNNING.store(false, Ordering::SeqCst);
    }
}

async fn load_set(
    State(state): State<AppState>,
    auth: AdminAuth,
    Json(body): Json<LoadBody>,
) -> impl IntoResponse {
    let Some((name, sub)) = SETS.iter().find(|(name, _)| *name == body.set.trim()) else {
        return err(
            StatusCode::UNPROCESSABLE_ENTITY,
            "set must be curated | world | world-nonparty",
        );
    };
    let Some((packs, xtask)) = release_paths() else {
        return err(
            StatusCode::SERVICE_UNAVAILABLE,
            "release packs directory unknown",
        );
    };
    let dir = set_dir(&packs, sub);
    if pack_files(&dir) == 0 || !xtask.is_file() {
        return err(
            StatusCode::NOT_FOUND,
            format!("no {name} packs in this release"),
        );
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        return err(StatusCode::CONFLICT, "a pack load is already running");
    }
    let guard = RunGuard;
    let before = authority_count(&state);
    let actor = format!("staff:{}", auth.user.user_id);
    let store_root = state.store_root.clone();
    let run_actor = actor.clone();
    let output = tokio::task::spawn_blocking(move || {
        Command::new(&xtask)
            .arg("packs-load")
            .arg("--packs")
            .arg(&dir)
            .arg("--store")
            .arg(&store_root)
            .arg("--actor")
            .arg(&run_actor)
            .output()
    })
    .await;
    drop(guard);
    let output = match output {
        Ok(Ok(output)) => output,
        Ok(Err(e)) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("spawn: {e}")),
        Err(e) => return err(StatusCode::INTERNAL_SERVER_ERROR, format!("join: {e}")),
    };
    let after = authority_count(&state);
    let ok = output.status.success();
    let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&output.stderr).into_owned();
    if let Ok(conn) = db::open(&Store::new(&state.store_root)) {
        let reason = format!(
            "set={name} ok={ok} authorities {} -> {}",
            before.unwrap_or(-1),
            after.unwrap_or(-1)
        );
        let _ = db::insert_audit(
            &conn,
            unix_now(),
            &actor,
            "packs_load",
            "packs",
            0,
            "",
            "",
            &reason,
        );
    }
    let status = if ok {
        StatusCode::OK
    } else {
        StatusCode::UNPROCESSABLE_ENTITY
    };
    (
        status,
        Json(serde_json::json!({
            "ok": ok,
            "set": name,
            "before": before,
            "after": after,
            "stdout": stdout,
            "stderr": stderr,
        })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sets_load_curated_first_and_map_to_release_dirs() {
        assert_eq!(SETS[0].0, "curated");
        let packs = Path::new("/srv/apostille/releases/abc/packs");
        assert_eq!(
            set_dir(packs, ""),
            Path::new("/srv/apostille/releases/abc/packs")
        );
        assert_eq!(
            set_dir(packs, "world"),
            Path::new("/srv/apostille/releases/abc/packs/world")
        );
    }

    #[test]
    fn pack_files_counts_json_only() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.json"), "{}").unwrap();
        std::fs::write(dir.path().join("README.md"), "x").unwrap();
        std::fs::create_dir(dir.path().join("world")).unwrap();
        assert_eq!(pack_files(dir.path()), 1);
        assert_eq!(pack_files(&dir.path().join("missing")), 0);
    }
}
