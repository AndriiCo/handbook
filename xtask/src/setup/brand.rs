//! Licensed brand assets: the provider logos and the Vogun face.
//!
//! The operator reads each owner's terms, ticks acceptance in the
//! wizard (recorded in the setup file with name, time, terms version
//! and each file's sha256) and drops the files into `private/brand/`
//! (git-ignored). Builds take only files whose hash matches a recorded
//! acceptance; anything else is left out and the login page falls back
//! to text buttons. The tool never accepts terms itself.

use std::path::{Path, PathBuf};

use sha2::{Digest as _, Sha256};

use super::file::{BrandFile, Licence, SetupFile};

pub struct Asset {
    pub id: &'static str,
    pub owner: &'static str,
    pub terms: &'static str,
    pub terms_url: &'static str,
    /// What the terms allow here, in one line (read the terms first).
    pub permits: &'static str,
    /// Where the file comes from.
    pub get: &'static str,
    pub files: &'static [&'static str],
}

pub const ASSETS: &[Asset] = &[
    Asset {
        id: "apple",
        owner: "Apple",
        terms: "Sign in with Apple: Human Interface Guidelines and the Apple Developer Program License Agreement",
        terms_url: "https://developer.apple.com/design/human-interface-guidelines/sign-in-with-apple",
        permits: "The Apple logo inside a Sign in with Apple button only, unmodified, at the sizes and colours the guidelines set.",
        get: "https://developer.apple.com/design/resources/ (Sign in with Apple logo SVG)",
        files: &["apple.svg"],
    },
    Asset {
        id: "google",
        owner: "Google",
        terms: "Sign in with Google branding guidelines",
        terms_url: "https://developers.google.com/identity/branding-guidelines",
        permits: "The standard Google 'G' in a sign-in button following the button rules; no altered colours or shapes.",
        get: "https://developers.google.com/identity/branding-guidelines (signin assets)",
        files: &["google.svg"],
    },
    Asset {
        id: "meta",
        owner: "Meta",
        terms: "Meta brand resources and usage guidelines",
        terms_url: "https://www.meta.com/brand/resources/",
        permits: "The Meta logo to name the Meta models the product uses, per the usage list; some uses need Meta brand review.",
        get: "https://www.meta.com/brand/resources/meta/company-brand/ (accept the guidelines, then download)",
        files: &["meta.svg"],
    },
    Asset {
        id: "fido",
        owner: "FIDO Alliance",
        terms: "Passkey Icon Usage Guidelines and Passkey Icon Usage Agreement",
        terms_url: "https://fidoalliance.org/get-the-passkey-icon/",
        permits: "The passkey icon next to passkey sign-in, unmodified; never as the product's own brand.",
        get: "https://fidoalliance.org/get-the-passkey-icon/ (form, then the icon pack)",
        files: &["passkey.svg"],
    },
    Asset {
        id: "vogun",
        owner: "Vogun font foundry",
        terms: "The operator's Vogun webfont licence",
        terms_url: "",
        permits: "Self-hosted web use of the purchased weights, served unmodified (no subsetting or conversion).",
        get: "The operator's licence download (WOFF2 files)",
        files: &["Vogun-Medium.woff2", "Vogun-MediumItalic.woff2"],
    },
];

pub fn asset(id: &str) -> Option<&'static Asset> {
    ASSETS.iter().find(|a| a.id == id)
}

/// The single file name a drop may use, or `None` for anything else.
pub fn allowed_file(name: &str) -> Option<&'static str> {
    ASSETS
        .iter()
        .flat_map(|a| a.files.iter().copied())
        .find(|f| *f == name)
}

pub fn dir(repo: &Path, file: &SetupFile) -> PathBuf {
    repo.join(file.brand.dir.as_deref().unwrap_or("private/brand"))
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Record the operator's acceptance of one asset's terms, with the
/// hashes of the files present now.
pub fn accept(
    repo: &Path,
    file: &mut SetupFile,
    id: &str,
    terms_version: &str,
    now_rfc3339: &str,
) -> Result<(), String> {
    let asset = asset(id).ok_or_else(|| format!("unknown brand asset {id:?}"))?;
    if file.operator.name.trim().is_empty() {
        return Err("set the operator name first (it is recorded with the acceptance)".into());
    }
    let dir = dir(repo, file);
    let files = asset
        .files
        .iter()
        .filter_map(|name| {
            std::fs::read(dir.join(name)).ok().map(|bytes| BrandFile {
                name: (*name).to_string(),
                sha256: sha256_hex(&bytes),
            })
        })
        .collect();
    file.brand.licences.retain(|l| l.asset != id);
    file.brand.licences.push(Licence {
        asset: id.to_string(),
        terms: asset.terms.to_string(),
        terms_url: asset.terms_url.to_string(),
        terms_version: terms_version.to_string(),
        accepted_by: file.operator.name.clone(),
        accepted_at: now_rfc3339.to_string(),
        files,
    });
    Ok(())
}

/// Files a build may ship: present, and hashed in a recorded acceptance.
pub fn licensed_files(repo: &Path, file: &SetupFile) -> Vec<PathBuf> {
    let dir = dir(repo, file);
    file.brand
        .licences
        .iter()
        .flat_map(|l| l.files.iter())
        .filter_map(|f| {
            let path = dir.join(&f.name);
            let bytes = std::fs::read(&path).ok()?;
            (allowed_file(&f.name).is_some() && sha256_hex(&bytes) == f.sha256).then_some(path)
        })
        .collect()
}

/// RFC 3339 UTC for a unix time (no date crate: civil-from-days).
pub fn rfc3339(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_accepted_unchanged_files_ship() {
        let repo = tempfile::tempdir().unwrap();
        let mut file = SetupFile::template();
        let brand = repo.path().join("private/brand");
        std::fs::create_dir_all(&brand).unwrap();
        std::fs::write(brand.join("google.svg"), "<svg>synthetic</svg>").unwrap();
        std::fs::write(brand.join("apple.svg"), "<svg>synthetic apple</svg>").unwrap();
        assert!(
            accept(
                repo.path(),
                &mut file,
                "google",
                "2026-11",
                "2026-11-03T00:00:00Z"
            )
            .is_err(),
            "needs an operator name"
        );
        file.operator.name = "Synthetic Operator".into();
        accept(
            repo.path(),
            &mut file,
            "google",
            "2026-11",
            "2026-11-03T00:00:00Z",
        )
        .unwrap();
        let shipped = licensed_files(repo.path(), &file);
        assert_eq!(
            shipped,
            vec![brand.join("google.svg")],
            "apple was never accepted"
        );
        std::fs::write(brand.join("google.svg"), "<svg>changed</svg>").unwrap();
        assert!(
            licensed_files(repo.path(), &file).is_empty(),
            "a changed file needs a new acceptance"
        );
        assert!(allowed_file("../etc/passwd").is_none());
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        assert_eq!(rfc3339(1_793_664_000), "2026-11-03T00:00:00Z");
    }
}
