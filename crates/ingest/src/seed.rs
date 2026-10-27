//! Watch seed shape: ONE HCCH URI plus the cited-URL watch list.
//!
//! The real seed (1 HCCH URI + 10 URLs) is author data (roadmap §5);
//! [`Seed::synthetic`] is the offline fallback for dev/CI. URIs are data,
//! never code: callers pass them to [`crate::fetch::fetch_url`].

/// One watched URL and the authority slug it evidences.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeedUrl {
    /// Authority slug owning this URL (e.g. `usa-wa-sos`).
    pub authority_slug: String,
    /// URL to fetch.
    pub uri: String,
}

/// Thin-watch seed: one HCCH status URI plus cited-URL watch entries.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seed {
    /// The single HCCH Convention-status URI (F2).
    pub hcch_uri: String,
    /// Cited-URL watch list (F1); the Day-2 seed carries 10.
    pub urls: Vec<SeedUrl>,
}

impl Seed {
    /// Synthetic offline seed: one HCCH-shaped URI + exactly 10 watch
    /// URLs under `example.com`. Never fetched in tests; never committed
    /// as real data.
    pub fn synthetic() -> Self {
        let urls = (1..=10)
            .map(|n| SeedUrl {
                authority_slug: format!("synthetic-{n:02}"),
                uri: format!("https://example.com/synthetic/{n}"),
            })
            .collect();
        Self {
            hcch_uri: "https://example.com/hcch/status-table".to_string(),
            urls,
        }
    }

    /// True when the seed has its HCCH URI and at least one watch URL.
    pub fn is_complete(&self) -> bool {
        !self.hcch_uri.is_empty() && !self.urls.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn synthetic_seed_has_one_hcch_uri_and_ten_urls() {
        let seed = Seed::synthetic();
        assert!(seed.is_complete());
        assert!(!seed.hcch_uri.is_empty());
        assert_eq!(seed.urls.len(), 10);
        assert!(
            seed.urls
                .iter()
                .all(|u| !u.authority_slug.is_empty() && !u.uri.is_empty()),
            "every entry carries a slug and a uri"
        );
    }
}
