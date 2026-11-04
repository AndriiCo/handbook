//! Licensed brand assets: the provider logos and the Vogun face.
//!
//! The operator reads each owner's terms, ticks acceptance in the
//! wizard (recorded in the setup file with name, time, terms version
//! and each file's sha256) and drops the files into `private/brand/`
//! (git-ignored), or, once acceptance is ticked, has the wizard
//! **Fetch** them from their source ([`fetch_file`]; the hash goes into
//! the acceptance). Builds take only files whose hash matches a
//! recorded acceptance; anything else is left out and the login page
//! falls back to text buttons. The tool never accepts terms itself.

use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::LazyLock;
use std::time::Duration;

use regex::Regex;
use sha2::{Digest as _, Sha256};

use super::file::{BrandFile, BrandSource, Licence, SetupFile};

/// A download the wizard reads at most (a zip of every variant).
pub const MAX_DOWNLOAD: u64 = 20 * 1024 * 1024;
/// A brand file the wizard writes at most (the upload cap too).
pub const MAX_FILE: usize = 2 * 1024 * 1024;

/// Sources downloaded and opened by hand on 2026-11-04; the setup
/// file's `brand.sources` overrides them. (file, url, entry inside)
/// - Google: the sign-in assets zip from its branding guidelines page;
///   the neutral square "G" (a Figma export: its conic gradient is a
///   paint-only `<foreignObject>`, see [`svg_problem`]).
/// - Apple: the "Sign in with Apple Logo" disk image from Apple Design
///   Resources, read by [`super::dmg`]; the left-aligned black logo at
///   Medium, the size for 44-pt buttons with a title.
/// - FIDO: the "FIDO Passkey mark A" pack linked from FIDO's passkey
///   icon download page; the black mark. FIDO's agreement is formed by
///   completing that page's form, so the acceptance names it.
///
/// Meta's logo sits behind accepting its guidelines on its site, and
/// Vogun is a purchased licence: both stay manual.
pub const DEFAULT_SOURCES: &[(&str, &str, &str)] = &[
    (
        "google.svg",
        "https://developers.google.com/static/identity/images/signin-assets.zip",
        "Theme=Neutral, Show text=No, Shape=Square, Platform=Android+Web.svg",
    ),
    (
        "apple.svg",
        "https://devimages-cdn.apple.com/design/resources/download/Logo-Sign-in-with-Apple.dmg",
        "Logo - SIWA - Left-aligned - Black - Medium.svg",
    ),
    (
        "passkey.svg",
        "https://fidoalliance.org/wp-content/uploads/2024/01/FIDO_Passkey_mark_A.zip",
        "FIDO_Passkey_mark_A_black.svg",
    ),
];

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
        get: "https://developer.apple.com/design/resources/ (Sign in with Apple Logo, a .dmg the wizard opens)",
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
        terms: "Passkey Icon Usage Guidelines and Passkey Icon Usage Agreement (formed by completing the form at https://fidoalliance.org/passkey-download/: do that first)",
        terms_url: "https://fidoalliance.org/get-the-passkey-icon/",
        permits: "The passkey icon next to passkey sign-in, unmodified; never as the product's own brand.",
        get: "https://fidoalliance.org/passkey-download/ (complete the form, then the FIDO Passkey mark A pack)",
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

/// Where Fetch downloads `name`: the file's own source, else the
/// checked default, else none (manual).
pub fn source(file: &SetupFile, name: &str) -> Option<BrandSource> {
    file.brand
        .sources
        .get(name)
        .cloned()
        .or_else(|| default_source(name))
}

pub fn default_source(name: &str) -> Option<BrandSource> {
    DEFAULT_SOURCES
        .iter()
        .find(|(f, _, _)| *f == name)
        .map(|(_, url, pick)| BrandSource {
            url: (*url).to_string(),
            pick: Some((*pick).to_string()),
        })
}

/// Why an SVG may not ship, if it may not: scripts, event handlers,
/// `<foreignObject>` (except paint-only ones, [`foreign_objects_are_paint`])
/// and anything fetched from elsewhere.
pub fn svg_problem(svg: &str) -> Option<&'static str> {
    static EVENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r#"[\s/"']on[a-z]+\s*="#).unwrap());
    static LINK: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"href\s*=\s*["']?\s*(https?:|//|javascript:)|url\(\s*["']?\s*(https?:|//)"#)
            .unwrap()
    });
    let lower = svg.to_ascii_lowercase();
    if !lower.contains("<svg") {
        Some("it is not an SVG")
    } else if lower.contains("<script") {
        Some("it carries a <script>")
    } else if lower.contains("<foreignobject") && !foreign_objects_are_paint(&lower) {
        Some("it carries a <foreignObject>")
    } else if EVENT.is_match(&lower) {
        Some("it carries an on… event attribute")
    } else if LINK.is_match(&lower) {
        Some("it links to an outside resource")
    } else {
        None
    }
}

/// True when every `<foreignObject>` holds only empty, inline-styled
/// XHTML `<div>`s with no `url(`, `@import` or `expression`. That is
/// how Figma exports paint SVG cannot draw (Google's 2025 "G" is a
/// conic gradient): no text, script or link, and the logos render
/// through `<img src>`, where nothing in an SVG runs or loads. Any other
/// content inside a `<foreignObject>` keeps the SVG out. `lower` is the
/// lowercased SVG.
fn foreign_objects_are_paint(lower: &str) -> bool {
    static PAINT: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(
            r#"^(\s*<div xmlns="http://www\.w3\.org/1999/xhtml" style="[^"<>]*"\s*>\s*</div>)+\s*$"#,
        )
        .unwrap()
    });
    let mut rest = lower;
    while let Some(open) = rest.find("<foreignobject") {
        let after = &rest[open..];
        let (Some(tag_end), Some(close)) = (after.find('>'), after.find("</foreignobject>")) else {
            return false;
        };
        if close < tag_end {
            return false;
        }
        let inner = &after[tag_end + 1..close];
        if !PAINT.is_match(inner)
            || ["url(", "@import", "expression"]
                .iter()
                .any(|w| inner.contains(w))
        {
            return false;
        }
        rest = &after[close + "</foreignobject>".len()..];
    }
    true
}

/// `bytes` must be what a brand file called `name` holds. Fetched and
/// uploaded files meet the same bar.
pub fn check_content(name: &str, bytes: &[u8]) -> Result<(), String> {
    if bytes.is_empty() || bytes.len() > MAX_FILE {
        return Err(format!("{name} must be 1 byte to 2 MB"));
    }
    if name.ends_with(".svg") {
        let text = std::str::from_utf8(bytes).map_err(|_| format!("{name} is not UTF-8 text"))?;
        if let Some(why) = svg_problem(text) {
            return Err(format!("refused {name}: {why}"));
        }
        Ok(())
    } else if bytes.starts_with(b"wOF2") {
        Ok(())
    } else {
        Err(format!("{name} is not a WOFF2 font"))
    }
}

/// The entry called `want` (its path, or a path ending in `/want`) from
/// a zip, read with `unzip` (no zip crate: one tool every Mac and Linux
/// box has), at most [`MAX_FILE`] bytes. Nothing is unpacked to disk.
fn unzip_entry(zip: &[u8], want: &str) -> Result<Vec<u8>, String> {
    let dir = tempfile::tempdir().map_err(|e| e.to_string())?;
    let path = dir.path().join("download.zip");
    std::fs::write(&path, zip).map_err(|e| e.to_string())?;
    let list = Command::new("unzip")
        .arg("-Z1")
        .arg(&path)
        .output()
        .map_err(|e| format!("run unzip (is it installed?): {e}"))?;
    if !list.status.success() {
        return Err("the download is not a readable zip".into());
    }
    let names = String::from_utf8_lossy(&list.stdout);
    let suffix = format!("/{want}");
    let found: Vec<&str> = names
        .lines()
        .filter(|n| !n.ends_with('/') && (*n == want || n.ends_with(&suffix)))
        .collect();
    let entry = match found.as_slice() {
        [one] => *one,
        [] => return Err(format!("no {want:?} in the zip")),
        _ => {
            return Err(format!(
                "{want:?} matches {} entries; name its path",
                found.len()
            ));
        }
    };
    if entry.starts_with('-') {
        return Err(format!("refused the zip entry {entry:?}"));
    }
    // unzip reads names as wildcards: escape them.
    let mut pattern = String::new();
    for c in entry.chars() {
        if "*?[]\\".contains(c) {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    let mut child = Command::new("unzip")
        .arg("-p")
        .arg(&path)
        .arg(pattern)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("run unzip: {e}"))?;
    let mut out = Vec::new();
    child
        .stdout
        .take()
        .ok_or("unzip stdout")?
        .take(MAX_FILE as u64 + 1)
        .read_to_end(&mut out)
        .map_err(|e| e.to_string())?;
    let _ = child.kill();
    let _ = child.wait();
    Ok(out)
}

/// Download, pick and check one brand file and write it to the brand
/// directory. Refused unless the operator has accepted its owner's
/// terms. `get` downloads a URL (the network seam: [`download`]).
/// Returns the file's sha256 for [`record`].
pub fn fetch_file(
    repo: &Path,
    file: &SetupFile,
    name: &str,
    get: &dyn Fn(&str) -> Result<Vec<u8>, String>,
) -> Result<String, String> {
    let name = allowed_file(name).ok_or("not a brand file name this build uses")?;
    let asset = ASSETS
        .iter()
        .find(|a| a.files.contains(&name))
        .expect("allowed files belong to an asset");
    if !file.brand.licences.iter().any(|l| l.asset == asset.id) {
        return Err(format!(
            "tick acceptance of {}'s terms first; this tool never accepts them for you",
            asset.owner
        ));
    }
    let src = source(file, name).ok_or("no source URL for this file: download it by hand")?;
    if !src.url.starts_with("https://") {
        return Err("the source must be an https:// URL".into());
    }
    let bytes = get(&src.url)?;
    let pick = src.pick.as_deref().unwrap_or(name);
    let bytes = if bytes.starts_with(b"PK\x03\x04") {
        unzip_entry(&bytes, pick)?
    } else if super::dmg::is_dmg(&bytes) {
        super::dmg::file(&bytes, pick, MAX_FILE)?
    } else {
        bytes
    };
    check_content(name, &bytes)?;
    let dir = dir(repo, file);
    std::fs::create_dir_all(&dir)
        .and_then(|()| std::fs::write(dir.join(name), &bytes))
        .map_err(|e| e.to_string())?;
    Ok(sha256_hex(&bytes))
}

/// Put a fetched file's hash into its asset's recorded acceptance.
pub fn record(file: &mut SetupFile, name: &str, sha256: &str) -> Result<(), String> {
    let asset = ASSETS
        .iter()
        .find(|a| a.files.contains(&name))
        .ok_or("not a brand file name this build uses")?;
    let licence = file
        .brand
        .licences
        .iter_mut()
        .find(|l| l.asset == asset.id)
        .ok_or("the acceptance was withdrawn while fetching")?;
    licence.files.retain(|f| f.name != name);
    licence.files.push(BrandFile {
        name: name.to_string(),
        sha256: sha256.to_string(),
    });
    Ok(())
}

/// GET `url` over HTTPS only (redirects too), 30 s, [`MAX_DOWNLOAD`].
pub fn download(url: &str) -> Result<Vec<u8>, String> {
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let _ = rustls::crypto::CryptoProvider::install_default(provider);
    let agent: ureq::Agent = ureq::Agent::config_builder()
        .https_only(true)
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(30)))
        .build()
        .into();
    let mut response = agent.get(url).call().map_err(|e| format!("{url}: {e}"))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(format!("{url}: HTTP {status}"));
    }
    response
        .body_mut()
        .with_config()
        .limit(MAX_DOWNLOAD)
        .read_to_vec()
        .map_err(|e| format!("{url}: {e} (the cap is 20 MB)"))
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

    #[test]
    fn svgs_with_scripts_handlers_foreign_objects_or_outside_links_are_refused() {
        let ok = [
            r##"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink"><path d="M0 0" fill="url(#g)"/><use xlink:href="#a"/></svg>"##,
            r#"<svg><image href="data:image/png;base64,AAAA"/><g font-family="x" data-on="1"/></svg>"#,
        ];
        for svg in ok {
            assert_eq!(svg_problem(svg), None, "{svg}");
        }
        let bad = [
            ("<html></html>", "not an SVG"),
            ("<svg><SCRIPT>alert(1)</SCRIPT></svg>", "<script>"),
            (r#"<svg onload="alert(1)"></svg>"#, "event"),
            ("<svg>\n<g\nONCLICK = 'x'/></svg>", "event"),
            (
                "<svg><foreignObject><div/></foreignObject></svg>",
                "foreignObject",
            ),
            (
                r#"<svg><a href="https://evil.example/">x</a></svg>"#,
                "outside",
            ),
            (
                r#"<svg><use xlink:href = 'http://evil.example/s.svg#a'/></svg>"#,
                "outside",
            ),
            (
                r#"<svg><image href="//evil.example/p.png"/></svg>"#,
                "outside",
            ),
            (
                r#"<svg><a href="javascript:alert(1)">x</a></svg>"#,
                "outside",
            ),
            (
                r#"<svg><rect style="fill:url( 'https://evil.example/#p')"/></svg>"#,
                "outside",
            ),
        ];
        for (svg, why) in bad {
            let got = svg_problem(svg).unwrap_or_else(|| panic!("accepted {svg}"));
            assert!(got.contains(why), "{svg}: {got}");
        }
    }

    #[test]
    fn fetch_needs_acceptance_then_picks_checks_writes_and_records() {
        let repo = tempfile::tempdir().unwrap();
        let mut file = SetupFile::template();
        file.operator.name = "Synthetic Operator".into();
        let plain = |_: &str| Ok(b"<svg>synthetic G</svg>".to_vec());
        let err = fetch_file(repo.path(), &file, "google.svg", &plain).unwrap_err();
        assert!(err.contains("tick acceptance"), "{err}");
        accept(
            repo.path(),
            &mut file,
            "google",
            "2026-11",
            "2026-11-04T00:00:00Z",
        )
        .unwrap();
        // The default source is Google's bundle; a plain SVG answer is used as is.
        assert!(
            source(&file, "google.svg")
                .unwrap()
                .url
                .ends_with("signin-assets.zip")
        );
        let sha = fetch_file(repo.path(), &file, "google.svg", &|url: &str| {
            assert!(url.starts_with("https://developers.google.com/"), "{url}");
            Ok(b"<svg>synthetic G</svg>".to_vec())
        })
        .unwrap();
        assert_eq!(sha, sha256_hex(b"<svg>synthetic G</svg>"));
        assert!(
            licensed_files(repo.path(), &file).is_empty(),
            "not recorded yet"
        );
        record(&mut file, "google.svg", &sha).unwrap();
        assert_eq!(
            licensed_files(repo.path(), &file),
            vec![repo.path().join("private/brand/google.svg")]
        );
        // Failures say why and leave the file alone.
        let refused = |_: &str| Ok(br#"<svg onload="x()"></svg>"#.to_vec());
        let err = fetch_file(repo.path(), &file, "google.svg", &refused).unwrap_err();
        assert!(err.contains("refused google.svg"), "{err}");
        let down = |_: &str| Err("https://x: HTTP 404".to_string());
        assert!(
            fetch_file(repo.path(), &file, "google.svg", &down)
                .unwrap_err()
                .contains("404")
        );
        assert_eq!(licensed_files(repo.path(), &file).len(), 1);
        file.brand.sources.insert(
            "google.svg".into(),
            BrandSource {
                url: "http://example.test/g.svg".into(),
                pick: None,
            },
        );
        assert!(
            fetch_file(repo.path(), &file, "google.svg", &plain)
                .unwrap_err()
                .contains("https://")
        );
        // No source and no default (Meta's logo): manual.
        accept(
            repo.path(),
            &mut file,
            "meta",
            "2026-11",
            "2026-11-04T00:00:00Z",
        )
        .unwrap();
        let err = fetch_file(repo.path(), &file, "meta.svg", &plain).unwrap_err();
        assert!(err.contains("by hand"), "{err}");
        assert!(fetch_file(repo.path(), &file, "../x.svg", &plain).is_err());
    }

    /// Google's 2025 "G" is a Figma export: its conic gradient is a
    /// `<foreignObject>` holding one empty styled XHTML div. That passes;
    /// anything else inside a `<foreignObject>` does not.
    #[test]
    fn paint_only_foreign_objects_pass_and_nothing_else_inside_does() {
        let paint = r#"<svg xmlns="http://www.w3.org/2000/svg"><g clip-path="url(#c)"><foreignObject x="-1" y="-1" width="2" height="2"><div xmlns="http://www.w3.org/1999/xhtml" style="background:conic-gradient(from 90deg,rgba(255, 70, 65, 1) 0deg,rgba(49, 134, 255, 1) 39deg);height:100%;width:100%;opacity:1"></div></foreignObject></g></svg>"#;
        assert_eq!(svg_problem(paint), None);
        let twice = paint.replace(
            "</foreignObject>",
            "</foreignObject><foreignObject><div xmlns=\"http://www.w3.org/1999/xhtml\" style=\"opacity:1\"></div></foreignObject>",
        );
        assert_eq!(svg_problem(&twice), None);
        for bad in [
            r#"<svg><foreignObject><div xmlns="http://www.w3.org/1999/xhtml" style="">hi</div></foreignObject></svg>"#,
            r#"<svg><foreignObject><div xmlns="http://www.w3.org/1999/xhtml" style="background:url(data:image/png;base64,AA)"></div></foreignObject></svg>"#,
            r#"<svg><foreignObject><div xmlns="http://www.w3.org/1999/xhtml" style=""></div><img src="x"/></foreignObject></svg>"#,
            r#"<svg><foreignObject><div xmlns="http://www.w3.org/1999/xhtml" style="x"><div></div></div></foreignObject></svg>"#,
            r#"<svg><foreignObject><div xmlns="http://www.w3.org/1999/xhtml" style="x"></div></svg>"#,
            r#"<svg><foreignObject/></svg>"#,
            r#"<svg><foreignObject><div style="x"></div></foreignObject></svg>"#,
        ] {
            let why = svg_problem(bad).unwrap_or_else(|| panic!("accepted {bad}"));
            assert!(why.contains("foreignObject"), "{bad}: {why}");
        }
    }

    /// Apple's logo comes as a disk image: Fetch opens it and keeps the
    /// one named file.
    #[test]
    fn disk_images_give_up_the_one_named_file() {
        use super::super::dmg::tests::{hfs, udif};
        let svg = br#"<svg xmlns="http://www.w3.org/2000/svg"><path d="M0 0"/></svg>"#;
        let dmg = udif(&hfs(&[
            ("Logo - SIWA - Left-aligned - White - Medium.svg", b"<svg/>"),
            ("Logo - SIWA - Left-aligned - Black - Medium.svg", svg),
        ]));
        let repo = tempfile::tempdir().unwrap();
        let mut file = SetupFile::template();
        file.operator.name = "Synthetic Operator".into();
        accept(
            repo.path(),
            &mut file,
            "apple",
            "2026-11",
            "2026-11-04T00:00:00Z",
        )
        .unwrap();
        let sha = fetch_file(repo.path(), &file, "apple.svg", &|url: &str| {
            assert!(url.ends_with("/Logo-Sign-in-with-Apple.dmg"), "{url}");
            Ok(dmg.clone())
        })
        .unwrap();
        assert_eq!(sha, sha256_hex(svg));
        assert_eq!(
            std::fs::read(repo.path().join("private/brand/apple.svg")).unwrap(),
            svg
        );
    }

    /// The three default sources, for real: each downloads, gives up its
    /// named file and passes the checks. Network, so run by hand:
    /// `cargo test -p xtask -- --ignored default_sources`. With
    /// `BRAND_DL_DIR` set, each download is read from
    /// `<dir>/<last URL segment>` instead (a proxy that re-signs TLS).
    #[test]
    #[ignore]
    fn default_sources_download_pick_and_pass() {
        let local = std::env::var("BRAND_DL_DIR").ok();
        for (name, url, pick) in DEFAULT_SOURCES {
            let bytes = match &local {
                Some(dir) => std::fs::read(Path::new(dir).join(url.rsplit('/').next().unwrap()))
                    .unwrap_or_else(|e| panic!("{name}: {e}")),
                None => download(url).unwrap_or_else(|e| panic!("{name}: {e}")),
            };
            let got = if bytes.starts_with(b"PK\x03\x04") {
                unzip_entry(&bytes, pick)
            } else {
                super::super::dmg::file(&bytes, pick, MAX_FILE)
            }
            .unwrap_or_else(|e| panic!("{name}: {e}"));
            check_content(name, &got).unwrap_or_else(|e| panic!("{name}: {e}"));
            eprintln!("{name}: {} bytes from {pick}", got.len());
        }
    }

    #[test]
    fn zips_give_up_the_one_named_entry() {
        if Command::new("unzip").arg("-v").output().is_err() {
            return; // no unzip on this box
        }
        let zip = include_bytes!("testdata/brand-bundle.zip");
        let repo = tempfile::tempdir().unwrap();
        let mut file = SetupFile::template();
        file.operator.name = "Synthetic Operator".into();
        accept(
            repo.path(),
            &mut file,
            "google",
            "2026-11",
            "2026-11-04T00:00:00Z",
        )
        .unwrap();
        let get = |_: &str| Ok(zip.to_vec());
        let fetch = |file: &mut SetupFile, pick: &str| {
            file.brand.sources.insert(
                "google.svg".into(),
                BrandSource {
                    url: "https://example.test/bundle.zip".into(),
                    pick: Some(pick.into()),
                },
            );
            fetch_file(repo.path(), file, "google.svg", &get)
        };
        fetch(&mut file, "Theme=Neutral, Show text=No, Shape=Square.svg").unwrap();
        let got = std::fs::read_to_string(repo.path().join("private/brand/google.svg")).unwrap();
        assert!(got.contains("#4285F4"), "{got}");
        fetch(&mut file, "Bundle/a[1].svg").unwrap();
        let got = std::fs::read_to_string(repo.path().join("private/brand/google.svg")).unwrap();
        assert_eq!(
            got.trim(),
            "<svg>glob</svg>",
            "wildcards in names are literal"
        );
        let err = fetch(&mut file, "scripted.svg").unwrap_err();
        assert!(err.contains("<script>"), "{err}");
        let err = fetch(&mut file, "missing.svg").unwrap_err();
        assert!(err.contains("no \"missing.svg\" in the zip"), "{err}");
        let err = fetch_file(repo.path(), &file, "google.svg", &|_: &str| {
            Ok(b"PK\x03\x04 not really a zip".to_vec())
        })
        .unwrap_err();
        assert!(err.contains("not a readable zip"), "{err}");
    }
}
