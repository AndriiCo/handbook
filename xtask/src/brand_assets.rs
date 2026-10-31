//! Operator brand-asset runbook (scoreboard row GP3).
//!
//! `cargo xtask brand-assets` prints the ordered download steps: which
//! official page to open, what to review and accept as Andrii Co., and
//! which file should land in `~/Downloads`.
//!
//! `cargo xtask brand-assets --check [--minutes <n>] [--downloads <dir>]`
//! then scans the downloads dir for files created inside the window and
//! reports, per file, which provider step it belongs to (by known name,
//! keyword, and — for ZIPs — by listing the archive contents).
//!
//! This task never downloads, renders, or installs artwork itself: the
//! operator clicks accept in their own browser, and the SVGs stay
//! operator-owned (never committed; see web/src/lib/assets/brand/README.md).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

/// A download step the operator performs in their own browser.
struct Step {
    provider: &'static str,
    page: &'static str,
    review_accept: &'static str,
    want: &'static str,
}

const STEPS: &[Step] = &[
    Step {
        provider: "Apple (Sign in with Apple logo)",
        page: "https://developer.apple.com/design/human-interface-guidelines/sign-in-with-apple",
        review_accept: "Read the button rules (title, logo, size); accept the Apple Developer Program License Agreement as Andrii Co. during enrollment.",
        want: "From https://developer.apple.com/design/resources/ download the Design Resources pack (DMG/ZIP with 'Design' in the name); keep only the Sign-in-with-Apple logo SVG.",
    },
    Step {
        provider: "Google (prefer no download)",
        page: "https://developers.google.com/identity/branding-guidelines",
        review_accept: "Read the button rules; accept the Google APIs Terms when enabling OAuth; complete brand verification in the Google Auth Platform console.",
        want: "No file needed: render the button with Google Identity Services. If assets are downloaded anyway, expect classic names like btn_google_signin_dark_normal_web.png.",
    },
    Step {
        provider: "Meta (company lockup; NOT a login button)",
        page: "https://www.meta.com/brand/resources/meta/company-brand/",
        review_accept: "Read the usage list; tick 'I have read and accept the applicable guidelines' as Andrii Co.; all Meta-logo use additionally needs brand-review approval via a Meta counterpart.",
        want: "Meta_Company-Lockup.zip. For a 'Continue with Facebook' button use the Facebook Login assets instead of this lockup.",
    },
    Step {
        provider: "FIDO (passkey icon)",
        page: "https://fidoalliance.org/get-the-passkey-icon/",
        review_accept: "Read the Passkey Icon Usage Guidelines; submit the download form, accepting the Passkey Icon Usage Agreement as Andrii Co.",
        want: "Passkey icon pack (ZIP with 'passkey' or 'fido' in the name); never use the icon standalone as the product brand.",
    },
];

/// Provider guess for a downloaded file name (lowercase comparison).
fn classify(file_name: &str) -> Option<&'static str> {
    let name = file_name.to_lowercase();
    if name.contains("meta") && name.contains("lockup") || name == "meta_company-lockup.zip" {
        return Some("Meta");
    }
    if name.contains("btn_google_signin") || name.contains("google") && name.contains("signin") {
        return Some("Google");
    }
    if name.contains("passkey") || name.contains("fido") {
        return Some("FIDO");
    }
    if name.contains("design") && name.contains("resource")
        || name.contains("sign in with apple")
        || name.contains("sign-in-with-apple")
    {
        return Some("Apple");
    }
    None
}

/// CLI entry: print the guide, or scan ~/Downloads with --check.
pub fn run(args: &[String]) {
    let mut check = false;
    let mut minutes: u64 = 180;
    let mut downloads: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--check" => check = true,
            "--minutes" => {
                i += 1;
                minutes = args.get(i).and_then(|s| s.parse().ok()).unwrap_or_else(|| {
                    eprintln!("brand-assets: --minutes needs a number");
                    std::process::exit(2);
                });
            }
            "--downloads" => {
                i += 1;
                downloads = args.get(i).map(|s| PathBuf::from(s.as_str()));
            }
            unknown => {
                eprintln!("brand-assets: unknown flag {unknown}");
                eprintln!(
                    "usage: cargo xtask brand-assets [--check [--minutes <n>] [--downloads <dir>]]"
                );
                std::process::exit(2);
            }
        }
        i += 1;
    }
    if check {
        run_check(minutes, downloads);
    } else {
        run_guide();
    }
}

fn run_guide() {
    println!("brand-assets: operator runbook (act as Andrii Co., keep every receipt)\n");
    println!("  [0] Use a company Apple ID / Google Cloud org / Meta account you are");
    println!("      authorized to bind. Save each confirmation (enrollment, form copy,");
    println!("      Meta approval reply) with the release paperwork.\n");
    for (n, step) in STEPS.iter().enumerate() {
        println!("  [{}] {}", n + 1, step.provider);
        println!("      visit:  {}", step.page);
        println!("      agree:  {}", step.review_accept);
        println!("      expect: {}", step.want);
        println!();
    }
    println!("  [5] Convert/keep one SVG per provider actually configured:");
    println!("      web/src/lib/assets/brand/google.svg | apple.svg | meta.svg");
    println!("      (operator-owned, never committed). Then verify:");
    println!("      cargo xtask brand-assets --check --minutes 60");
}

fn run_check(minutes: u64, downloads: Option<PathBuf>) {
    let dir = downloads.unwrap_or_else(default_downloads);
    let cutoff = SystemTime::now()
        .checked_sub(std::time::Duration::from_secs(minutes * 60))
        .unwrap_or(SystemTime::UNIX_EPOCH);
    let mut hits: Vec<(PathBuf, u64, Option<&'static str>)> = Vec::new();
    let entries = std::fs::read_dir(&dir).unwrap_or_else(|e| {
        eprintln!("brand-assets: cannot read {}: {e}", dir.display());
        std::process::exit(1);
    });
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let modified = entry
            .metadata()
            .and_then(|m| m.modified())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        if modified < cutoff {
            continue;
        }
        let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
        // WORKER-NOTE (builder): one-line borrow fix so the tree compiles;
        // the GP3 worker owns this file — rebase freely.
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        hits.push((path, size, classify(&name)));
    }
    hits.sort_by_key(|(p, _, _)| p.clone());
    println!(
        "brand-assets: {} file(s) modified in the last {minutes} min under {}",
        hits.len(),
        dir.display()
    );
    if hits.is_empty() {
        println!("  none — download first (steps above), then re-run --check.");
        return;
    }
    for (path, size, guess) in &hits {
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
        match guess {
            Some(provider) => println!("  [match: {provider}] {name} ({size} bytes)"),
            None => println!("  [unknown] {name} ({size} bytes)"),
        }
        if name.to_lowercase().ends_with(".zip") {
            list_zip(path);
        }
    }
    println!("\nExpected: Meta_Company-Lockup.zip, a Design-Resources pack,");
    println!("a passkey/fido pack, and/or btn_google_signin_* assets.");
    println!("Missing a match? The download may be older than --minutes,");
    println!("renamed by the browser (check for ' (1)' suffixes), or still");
    println!("in transit — re-run with a bigger --minutes value.");
}

/// Best-effort ZIP listing via the system `unzip` (absent = skip silently).
fn list_zip(path: &Path) {
    let out = Command::new("unzip").arg("-l").arg(path).output();
    match out {
        Ok(result) if result.status.success() => {
            let text = String::from_utf8_lossy(&result.stdout);
            for line in text.lines().skip(3) {
                let line = line.trim_end();
                if line.is_empty() || line.starts_with("---") || line.ends_with("files") {
                    continue;
                }
                if let Some(name) = line.split_whitespace().last() {
                    let mark = if name.ends_with(".svg") || name.ends_with(".png") {
                        " <-- artwork"
                    } else {
                        ""
                    };
                    println!("      in zip: {name}{mark}");
                }
            }
        }
        _ => println!("      in zip: (unzip unavailable; open it in Finder)"),
    }
}

fn default_downloads() -> PathBuf {
    match std::env::var("HOME") {
        Ok(home) => PathBuf::from(home).join("Downloads"),
        Err(_) => {
            eprintln!("brand-assets: HOME is unset; pass --downloads <dir>");
            std::process::exit(2);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::classify;

    #[test]
    fn known_names_classify() {
        assert_eq!(classify("Meta_Company-Lockup.zip"), Some("Meta"));
        assert_eq!(
            classify("btn_google_signin_dark_normal_web@2x.png"),
            Some("Google")
        );
        assert_eq!(classify("passkey-icon-pack.zip"), Some("FIDO"));
        assert_eq!(classify("Apple_Design_Resources.dmg"), Some("Apple"));
    }

    #[test]
    fn unknown_names_stay_unknown() {
        assert_eq!(classify("IMG_0042.png"), None);
        assert_eq!(classify("archive (1).zip"), None);
    }
}
