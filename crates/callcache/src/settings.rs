//! Where the caches live and how they behave, resolved in one place
//! for web, xtask and workers: flag > environment > sealed config >
//! default under the store dir.
//!
//! - Meta cache dir: `--meta-cache`, `META_CACHE_DIR`,
//!   `paths.meta_cache_dir`, default `<store>/meta-cache`;
//! - fetch cache dir: `--fetch-cache`, `FETCH_CACHE_DIR`,
//!   `paths.fetch_cache_dir`, default `<store>/fetch-cache`;
//! - mode (both caches): `--cache-mode`, `META_CACHE_MODE`,
//!   `meta.cache_mode`, default `record`;
//! - fetch mode only: `--fetch-cache-mode`, `FETCH_CACHE_MODE`;
//! - daily cap: `--daily-call-cap`, `META_DAILY_CALL_CAP`,
//!   `meta.daily_call_cap`, default 400.

use std::path::{Path, PathBuf};

use crate::budget::DEFAULT_DAILY_CALL_CAP;
use crate::{CacheMode, FetchCache, MetaCache};

/// Resolved cache settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CacheSettings {
    pub meta_dir: PathBuf,
    pub fetch_dir: PathBuf,
    pub mode: CacheMode,
    pub fetch_mode: CacheMode,
    pub daily_call_cap: u64,
}

/// One layer of overrides (`None` keeps what the layer below chose).
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub meta_dir: Option<String>,
    pub fetch_dir: Option<String>,
    pub mode: Option<String>,
    pub fetch_mode: Option<String>,
    pub daily_call_cap: Option<String>,
}

impl Overrides {
    /// The process environment layer.
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var(name).ok().filter(|v| !v.trim().is_empty());
        Self {
            meta_dir: var("META_CACHE_DIR"),
            fetch_dir: var("FETCH_CACHE_DIR"),
            mode: var("META_CACHE_MODE"),
            fetch_mode: var("FETCH_CACHE_MODE"),
            daily_call_cap: var("META_DAILY_CALL_CAP"),
        }
    }
}

impl CacheSettings {
    /// Compiled defaults under `store_root`.
    pub fn defaults(store_root: &Path) -> Self {
        Self {
            meta_dir: store_root.join("meta-cache"),
            fetch_dir: store_root.join("fetch-cache"),
            mode: CacheMode::Record,
            fetch_mode: CacheMode::Record,
            daily_call_cap: DEFAULT_DAILY_CALL_CAP,
        }
    }

    /// Apply one layer. A value that does not parse is reported in the
    /// returned warnings and ignored (fail safe: the layer below wins).
    #[must_use]
    pub fn apply(mut self, layer: &Overrides, warnings: &mut Vec<String>) -> Self {
        if let Some(dir) = &layer.meta_dir {
            self.meta_dir = PathBuf::from(dir.trim());
        }
        if let Some(dir) = &layer.fetch_dir {
            self.fetch_dir = PathBuf::from(dir.trim());
        }
        if let Some(text) = &layer.mode {
            match CacheMode::parse(text) {
                Some(mode) => {
                    self.mode = mode;
                    self.fetch_mode = mode;
                }
                None => warnings.push(format!(
                    "cache mode {text:?}: want record|replay-only|refresh"
                )),
            }
        }
        if let Some(text) = &layer.fetch_mode {
            match CacheMode::parse(text) {
                Some(mode) => self.fetch_mode = mode,
                None => warnings.push(format!(
                    "fetch cache mode {text:?}: want record|replay-only|refresh"
                )),
            }
        }
        if let Some(text) = &layer.daily_call_cap {
            match text.trim().parse::<u64>() {
                Ok(cap) => self.daily_call_cap = cap,
                Err(_) => warnings.push(format!("daily call cap {text:?}: want an integer ≥ 0")),
            }
        }
        self
    }

    /// The Meta call cache with the daily cap attached.
    pub fn meta_cache(&self) -> MetaCache {
        MetaCache::new(&self.meta_dir, self.mode).with_budget(self.daily_call_cap)
    }

    pub fn fetch_cache(&self) -> FetchCache {
        FetchCache::new(&self.fetch_dir, self.fetch_mode)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layers_apply_in_order_and_bad_values_keep_the_layer_below() {
        let mut warnings = Vec::new();
        let config = Overrides {
            meta_dir: Some("/srv/scratch/apostille/meta-cache".to_string()),
            mode: Some("refresh".to_string()),
            daily_call_cap: Some("50".to_string()),
            ..Overrides::default()
        };
        let flags = Overrides {
            mode: Some("sometimes".to_string()),
            fetch_mode: Some("replay-only".to_string()),
            ..Overrides::default()
        };
        let s = CacheSettings::defaults(Path::new("/var/lib/apostille"))
            .apply(&config, &mut warnings)
            .apply(&flags, &mut warnings);
        assert_eq!(
            s.meta_dir,
            PathBuf::from("/srv/scratch/apostille/meta-cache")
        );
        assert_eq!(s.fetch_dir, PathBuf::from("/var/lib/apostille/fetch-cache"));
        assert_eq!(s.mode, CacheMode::Refresh);
        assert_eq!(s.fetch_mode, CacheMode::ReplayOnly);
        assert_eq!(s.daily_call_cap, 50);
        assert_eq!(warnings.len(), 1);
    }
}
