//! Call-cache settings for every xtask path that calls Meta or
//! fetches URLs (`docs/ops/call-cache.md`).
//!
//! Precedence: compiled default under `--store` (`<store>/meta-cache`,
//! `<store>/fetch-cache`), then the sealed rows when the KEK and the
//! store database are at hand, then the environment
//! (`META_CACHE_DIR`, `FETCH_CACHE_DIR`, `META_CACHE_MODE`,
//! `FETCH_CACHE_MODE`, `META_DAILY_CALL_CAP`), then the flags
//! `--meta-cache`, `--fetch-cache`, `--cache-mode`,
//! `--fetch-cache-mode`, `--daily-call-cap`. On plant pass
//! `--meta-cache /srv/scratch/apostille/meta-cache --fetch-cache
//! /srv/scratch/apostille/fetch-cache` (or export the env pair).

use std::path::PathBuf;
use std::sync::Arc;

use callcache::{CacheSettings, FetchCache, MetaCache, Overrides};

use crate::flag;

/// Resolved settings for this invocation (warnings go to stderr).
pub(crate) fn settings(args: &[String]) -> CacheSettings {
    let store = PathBuf::from(flag(args, "store").unwrap_or_else(|| "store".to_string()));
    let mut warnings = Vec::new();
    let mut settings = CacheSettings::defaults(&store);
    if let Some(rows) = sealed_rows(&store) {
        let sealed = Overrides {
            meta_dir: rows.meta_cache_dir,
            fetch_dir: rows.fetch_cache_dir,
            mode: rows.cache_mode,
            fetch_mode: None,
            daily_call_cap: rows.daily_call_cap,
        };
        settings = settings.apply(&sealed, &mut warnings);
    }
    let flags = Overrides {
        meta_dir: flag(args, "meta-cache"),
        fetch_dir: flag(args, "fetch-cache"),
        mode: flag(args, "cache-mode"),
        fetch_mode: flag(args, "fetch-cache-mode"),
        daily_call_cap: flag(args, "daily-call-cap"),
    };
    let settings = settings
        .apply(&Overrides::from_env(), &mut warnings)
        .apply(&flags, &mut warnings);
    for warning in warnings {
        eprintln!("call cache: {warning}; ignored");
    }
    settings
}

/// Sealed call-cache rows, when the store and KEK are both present
/// (never creates a store: a missing database means no rows).
fn sealed_rows(store: &std::path::Path) -> Option<sealed_config::CallCacheRows> {
    let store = handbook_store::store::Store::new(store);
    if !store.db_path().is_file() {
        return None;
    }
    let (kek, _source) = sealed_config::load_kek().ok()?;
    let conn = handbook_store::db::open(&store).ok()?;
    Some(sealed_config::call_cache_rows(&conn, &kek))
}

/// The Meta call cache (daily cap attached) for this invocation.
pub(crate) fn meta_cache(args: &[String]) -> Arc<MetaCache> {
    let cache = settings(args).meta_cache();
    eprintln!(
        "meta cache: {} ({}, daily cap {})",
        cache.root().display(),
        cache.mode(),
        cache.budget().map_or(0, callcache::Budget::cap)
    );
    Arc::new(cache)
}

/// The URL fetch cache for this invocation.
pub(crate) fn fetch_cache(args: &[String]) -> Arc<FetchCache> {
    Arc::new(settings(args).fetch_cache())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_win_and_defaults_sit_under_the_store() {
        let args: Vec<String> = [
            "x",
            "--store",
            "/nonexistent/store",
            "--fetch-cache",
            "/srv/scratch/apostille/fetch-cache",
            "--cache-mode",
            "replay-only",
            "--daily-call-cap",
            "7",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        let s = settings(&args);
        assert_eq!(
            s.fetch_dir,
            PathBuf::from("/srv/scratch/apostille/fetch-cache")
        );
        assert_eq!(s.mode, callcache::CacheMode::ReplayOnly);
        assert_eq!(s.daily_call_cap, 7);
        if std::env::var_os("META_CACHE_DIR").is_none() {
            assert_eq!(s.meta_dir, PathBuf::from("/nonexistent/store/meta-cache"));
        }
    }
}
