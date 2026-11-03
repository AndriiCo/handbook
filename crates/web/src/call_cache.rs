//! Call-cache settings for web Meta calls (`docs/ops/call-cache.md`).
//!
//! Read per call (an admin change applies at once): compiled defaults
//! under the store dir, then the sealed rows (`paths.meta_cache_dir`,
//! `meta.cache_mode`, `meta.daily_call_cap`), then the process
//! environment (`META_CACHE_DIR`, `META_CACHE_MODE`), the same
//! precedence `META_API_KEY` has over `meta.api_key`. Production's
//! default lands in `/var/lib/apostille/meta-cache`, inside the
//! unit's `ReadWritePaths`.

use std::path::PathBuf;
use std::sync::Arc;

use callcache::{CacheSettings, MetaCache, Overrides};
use rusqlite::Connection;

/// Store root behind `conn` (`<root>/handbook.sqlite`).
fn store_root(conn: &Connection) -> PathBuf {
    conn.path()
        .map(PathBuf::from)
        .and_then(|db| db.parent().map(PathBuf::from))
        .filter(|root| !root.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("store"))
}

/// Resolved cache settings for one call.
pub(crate) fn settings(conn: &Connection, kek: Option<&config::Kek>) -> CacheSettings {
    let rows = kek
        .map(|kek| config::call_cache_rows(conn, kek))
        .unwrap_or_default();
    let sealed = Overrides {
        meta_dir: rows.meta_cache_dir,
        fetch_dir: rows.fetch_cache_dir,
        mode: rows.cache_mode,
        fetch_mode: None,
        daily_call_cap: rows.daily_call_cap,
    };
    let mut warnings = Vec::new();
    let settings = CacheSettings::defaults(&store_root(conn))
        .apply(&sealed, &mut warnings)
        .apply(&Overrides::from_env(), &mut warnings);
    for warning in warnings {
        tracing::warn!(target: "call_cache", "{warning}; using the default");
    }
    settings
}

/// The Meta call cache (with the daily-cap ledger) for one call.
pub(crate) fn meta_cache(conn: &Connection, kek: Option<&config::Kek>) -> Arc<MetaCache> {
    Arc::new(settings(conn, kek).meta_cache())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_cache_sits_next_to_the_store_database() {
        let dir = tempfile::tempdir().unwrap();
        let conn = Connection::open(dir.path().join("handbook.sqlite")).unwrap();
        let s = settings(&conn, None);
        // Env overrides are process-global; only assert when unset.
        if std::env::var_os("META_CACHE_DIR").is_none() {
            // SQLite reports the resolved path (macOS: /private/var/…).
            let parent = s.meta_dir.parent().unwrap().canonicalize().unwrap();
            assert_eq!(parent, dir.path().canonicalize().unwrap());
            assert_eq!(s.meta_dir.file_name().unwrap(), "meta-cache");
        }
        assert_eq!(s.fetch_dir.file_name().unwrap(), "fetch-cache");
    }
}
