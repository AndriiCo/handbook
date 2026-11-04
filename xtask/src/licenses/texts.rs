//! Licence texts: SPDX expressions, the vendored text library
//! (`docs/licenses/texts/`), copyright extraction, and matching a
//! package's own licence file against the canonical texts so a text
//! that differs only by its copyright lines is printed once.

use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::time::Duration;

use sha2::{Digest, Sha256};

/// Pinned SPDX licence list release the missing texts are fetched from.
pub(crate) const SPDX_TAG: &str = "v3.29.0";
const SPDX_RAW: &str = "https://raw.githubusercontent.com/spdx/license-list-data";

/// Under this many words a "licence file" is a pointer ("MIT", "see
/// LICENSE-APACHE"), not a text: it is neither matched nor printed.
const MIN_WORDS: usize = 20;
/// A file under this many words that holds little of any licence text
/// ("dual licensed under MIT or Apache-2.0, see LICENSE-MIT") points at
/// the real texts instead of being one.
const POINTER_WORDS: usize = 150;

const INDEX_HEADER: &str = "\
# Licence texts for licenses.html (`cargo xtask licenses`).
#
# One `<ID>.txt` per SPDX licence or exception id the BOM uses, plus
# hand-kept `LicenseRef-*` texts. SPDX texts are fetched from
# spdx/license-list-data at the tag in `xtask/src/licenses/texts.rs`
# the first time an id appears, and this index gains their names; commit
# both. LicenseRef-* entries are written by hand.
";

// ---- SPDX expressions ----------------------------------------------------

/// One token of an SPDX expression. The deprecated `/` separator
/// (`MIT/Apache-2.0`) reads as `OR`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Tok {
    Id(String),
    Exception(String),
    Op(&'static str),
    Open,
    Close,
}

pub(crate) fn tokens(expr: &str) -> Vec<Tok> {
    let spaced = expr
        .replace('(', " ( ")
        .replace(')', " ) ")
        .replace('/', " OR ");
    let mut out = Vec::new();
    let mut after_with = false;
    for word in spaced.split_whitespace() {
        let tok = match word.to_ascii_uppercase().as_str() {
            "(" => Tok::Open,
            ")" => Tok::Close,
            "AND" => Tok::Op("AND"),
            "OR" => Tok::Op("OR"),
            "WITH" => Tok::Op("WITH"),
            _ if after_with => Tok::Exception(word.to_string()),
            _ => Tok::Id(word.to_string()),
        };
        after_with = tok == Tok::Op("WITH");
        out.push(tok);
    }
    out
}

/// Licence ids and exception ids named by an expression.
pub(crate) fn ids(expr: &str) -> (BTreeSet<String>, BTreeSet<String>) {
    let mut licences = BTreeSet::new();
    let mut exceptions = BTreeSet::new();
    for tok in tokens(expr) {
        match tok {
            Tok::Id(id) => {
                licences.insert(id);
            }
            Tok::Exception(id) => {
                exceptions.insert(id);
            }
            _ => {}
        }
    }
    (licences, exceptions)
}

// ---- normalising and copyright lines ---------------------------------------

/// A line that opens with a copyright marker, after comment leaders.
fn copyright_lead(line: &str) -> Option<&str> {
    let t = line
        .trim()
        .trim_start_matches(['#', '*', '/', ';', '-', '!', '>'])
        .trim();
    let lower = t.to_lowercase();
    // "(c)" alone also opens list items ("(c) You must retain…" in
    // Apache-2.0): only a year makes it a notice.
    let marked = lower.starts_with("(c)") && has_year(t);
    (lower.starts_with("copyright") || marked || t.starts_with('©')).then_some(t)
}

/// Byte offset of a copyright notice that starts mid-line ("…derived
/// from the foo crate, Copyright (c) 2015 Bar"), if any.
fn midline_copyright(line: &str) -> Option<usize> {
    let lower = line.to_ascii_lowercase();
    let mut from = 0;
    while let Some(i) = lower[from..].find("copyright") {
        let at = from + i;
        let rest = lower[at + "copyright".len()..].trim_start();
        if at > 0 && (rest.starts_with("(c)") || rest.starts_with('©') || starts_with_year(rest)) {
            return Some(at);
        }
        from = at + "copyright".len();
    }
    None
}

fn starts_with_year(s: &str) -> bool {
    let b = s.as_bytes();
    b.len() >= 4
        && b[..4].iter().all(u8::is_ascii_digit)
        && (b.starts_with(b"19") || b.starts_with(b"20"))
        && b.get(4).is_none_or(|c| !c.is_ascii_digit())
}

fn has_year(s: &str) -> bool {
    let b = s.as_bytes();
    b.windows(4).enumerate().any(|(i, w)| {
        w.iter().all(u8::is_ascii_digit)
            && (w.starts_with(b"19") || w.starts_with(b"20"))
            && (i == 0 || !b[i - 1].is_ascii_digit())
            && b.get(i + 4).is_none_or(|c| !c.is_ascii_digit())
    })
}

const TEMPLATE_MARKERS: &[&str] = &[
    "<year>",
    "[yyyy]",
    "{yyyy}",
    "[year]",
    "<copyright",
    "[name of",
    "<owner",
    "{name of",
    "[fullname]",
    "<name of",
    "<author",
    "yyyy",
];

const NOT_A_HOLDER: &[&str] = &[
    "notice",
    "notices",
    "holder",
    "holders",
    "owner",
    "owners",
    "law",
    "laws",
    "and",
    "license",
    "licence",
    "licenses",
    "statement",
    "protection",
    "is",
    "of",
    "in",
    "to",
    "for",
    "by",
    "information",
    "file",
    "act",
];

/// A copyright notice worth showing: not a template, not prose.
fn real_notice(t: &str) -> bool {
    let lower = t.to_lowercase();
    if TEMPLATE_MARKERS.iter().any(|m| lower.contains(m)) {
        return false;
    }
    let marked = lower.contains("(c)") || t.contains('©');
    if marked || has_year(t) {
        // "Copyright (c)" alone (the rest wrapped away) says nothing.
        return lower
            .trim_start_matches("copyright")
            .replace("(c)", "")
            .replace('©', "")
            .chars()
            .any(char::is_alphanumeric);
    }
    // "Copyright The Rust Project Developers" is a notice;
    // "copyright notice and this permission…" is prose.
    let rest = t.get("copyright".len()..).unwrap_or("").trim_start();
    let first: String = rest.chars().take_while(|c| c.is_alphabetic()).collect();
    first.chars().next().is_some_and(char::is_uppercase)
        && !NOT_A_HOLDER.contains(&first.to_lowercase().as_str())
        && t.chars().any(char::is_lowercase)
}

/// Copyright notices in a text plus, per line, how much of it is
/// licence text: `None` keeps the line, `Some(n)` keeps its first `n`
/// bytes (0 drops it). Handles notices that open mid-line and holder
/// lists that wrap onto lines starting with a year.
struct Scan {
    keep: Vec<Option<usize>>,
    notices: Vec<String>,
}

fn scan(text: &str) -> Scan {
    let lines: Vec<&str> = text.lines().collect();
    let mut keep = vec![None; lines.len()];
    let mut notices: Vec<String> = Vec::new();
    let mut in_block = false;
    for (i, line) in lines.iter().enumerate() {
        let trimmed = line.trim();
        let notice = if let Some(t) = copyright_lead(line) {
            keep[i] = Some(0);
            Some(t.to_string())
        } else if trimmed
            .trim_end_matches('.')
            .eq_ignore_ascii_case("all rights reserved")
        {
            keep[i] = Some(0);
            continue;
        } else if in_block && starts_with_year(trimmed) {
            keep[i] = Some(0);
            // A wrapped holder line continues the notice above it when
            // that one was cut short ("Copyright (c)" + "2015 Foo").
            match notices.last_mut() {
                Some(prev) if !real_notice(prev) => {
                    prev.push(' ');
                    prev.push_str(trimmed);
                    continue;
                }
                _ => Some(format!("Copyright {trimmed}")),
            }
        } else if let Some(at) = midline_copyright(line) {
            keep[i] = Some(at);
            Some(line[at..].trim().to_string())
        } else {
            in_block = false;
            continue;
        };
        in_block = true;
        if let Some(n) = notice {
            notices.push(n);
        }
    }
    let notices = notices
        .into_iter()
        .filter(|n| real_notice(n))
        .map(|n| {
            let clean = n.split_whitespace().collect::<Vec<_>>().join(" ");
            if clean.chars().count() > 240 {
                clean.chars().take(239).collect::<String>() + "…"
            } else {
                clean
            }
        })
        .collect();
    Scan { keep, notices }
}

/// The licence-text part of each line (copyright notices cut out).
fn text_lines(text: &str) -> impl Iterator<Item = &str> {
    let keep = scan(text).keep;
    text.lines()
        .zip(keep)
        .filter_map(|(line, keep)| match keep {
            None => Some(line),
            Some(0) => None,
            Some(n) => Some(&line[..n]),
        })
}

/// Real copyright notices in a text (templates and prose dropped),
/// whitespace collapsed, in order of appearance.
pub(crate) fn copyright_lines(text: &str) -> Vec<String> {
    scan(text).notices
}

/// Lowercase alphanumeric words with copyright notices removed: two
/// copies of one licence under different holders normalise equal.
pub(crate) fn normalise(text: &str) -> Vec<String> {
    let mut words = Vec::new();
    for line in text_lines(text) {
        let lower: String = line
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect();
        words.extend(lower.split_whitespace().map(str::to_string));
    }
    words
}

fn grams(words: &[String]) -> HashSet<u64> {
    words
        .windows(3)
        .map(|w| {
            let mut h = std::hash::DefaultHasher::new();
            w.hash(&mut h);
            h.finish()
        })
        .collect()
}

/// Short stable key of normalised words (variant and notice anchors).
pub(crate) fn key(words: &[String]) -> String {
    let digest = Sha256::digest(words.join(" ").as_bytes());
    digest.iter().take(4).map(|b| format!("{b:02x}")).collect()
}

/// A text with its copyright notices removed and runs of blank lines
/// squeezed: how a variant licence is printed (the holders are in the
/// table rows).
pub(crate) fn without_copyrights(text: &str) -> String {
    let mut out = String::new();
    let mut blank = 0;
    for line in text_lines(text) {
        if line.trim().is_empty() {
            blank += 1;
            if blank > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank = 0;
        }
        out.push_str(line.trim_end());
        out.push('\n');
    }
    out.trim_end().to_string()
}

// ---- the text library -----------------------------------------------------

pub(crate) struct Text {
    pub(crate) name: String,
    pub(crate) body: String,
    grams: HashSet<u64>,
}

pub(crate) struct Library {
    dir: PathBuf,
    pub(crate) texts: BTreeMap<String, Text>,
    /// id → (name, source), as in `index.toml`.
    index: BTreeMap<String, (String, String)>,
    pub(crate) fetched: Vec<String>,
}

#[derive(serde::Deserialize)]
struct IndexEntry {
    name: String,
    source: String,
}

impl Library {
    pub(crate) fn load(dir: &Path) -> Result<Library, String> {
        let index_path = dir.join("index.toml");
        let raw = std::fs::read_to_string(&index_path)
            .map_err(|e| format!("read {}: {e}", index_path.display()))?;
        let parsed: BTreeMap<String, IndexEntry> =
            toml::from_str(&raw).map_err(|e| format!("parse {}: {e}", index_path.display()))?;
        let mut lib = Library {
            dir: dir.to_path_buf(),
            texts: BTreeMap::new(),
            index: BTreeMap::new(),
            fetched: Vec::new(),
        };
        for (id, entry) in parsed {
            let path = dir.join(format!("{id}.txt"));
            let body = std::fs::read_to_string(&path)
                .map_err(|e| format!("{id} is in index.toml but {}: {e}", path.display()))?;
            lib.add(&id, entry.name.clone(), body);
            lib.index.insert(id, (entry.name, entry.source));
        }
        Ok(lib)
    }

    fn add(&mut self, id: &str, name: String, body: String) {
        let grams = grams(&normalise(&body));
        self.texts
            .insert(id.to_string(), Text { name, body, grams });
    }

    /// Fetch every SPDX id the BOM names but the library lacks, write
    /// `<ID>.txt` and rewrite `index.toml`. `LicenseRef-*` ids cannot
    /// be fetched: a missing one is an error naming the file to add.
    pub(crate) fn ensure(
        &mut self,
        licences: &BTreeSet<String>,
        exceptions: &BTreeSet<String>,
    ) -> Result<(), String> {
        let missing: Vec<(&String, bool)> = licences
            .iter()
            .map(|id| (id, false))
            .chain(exceptions.iter().map(|id| (id, true)))
            .filter(|(id, _)| !self.texts.contains_key(*id))
            .collect();
        if missing.is_empty() {
            return Ok(());
        }
        let refs: Vec<&str> = missing
            .iter()
            .filter(|(id, _)| id.starts_with("LicenseRef-"))
            .map(|(id, _)| id.as_str())
            .collect();
        if !refs.is_empty() {
            return Err(format!(
                "no text for {}: add docs/licenses/texts/<id>.txt and an index.toml entry",
                refs.join(", ")
            ));
        }
        let agent = agent();
        for (id, exception) in missing {
            let (name, body) = fetch_spdx(&agent, id, exception)?;
            let path = self.dir.join(format!("{id}.txt"));
            let mut body = body.trim_end().to_string();
            body.push('\n');
            std::fs::write(&path, &body).map_err(|e| format!("write {}: {e}", path.display()))?;
            self.add(id, name.clone(), body);
            self.index.insert(
                id.clone(),
                (name, format!("spdx/license-list-data {SPDX_TAG}")),
            );
            self.fetched.push(id.clone());
        }
        self.write_index()
    }

    fn write_index(&self) -> Result<(), String> {
        let mut out = INDEX_HEADER.to_string();
        for (id, (name, source)) in &self.index {
            out.push_str(&format!(
                "\n[{}]\nname = {}\nsource = {}\n",
                toml_str(id),
                toml_str(name),
                toml_str(source)
            ));
        }
        let path = self.dir.join("index.toml");
        std::fs::write(&path, out).map_err(|e| format!("write {}: {e}", path.display()))
    }

    pub(crate) fn name<'a>(&'a self, id: &'a str) -> &'a str {
        self.texts.get(id).map_or(id, |t| t.name.as_str())
    }

    /// Where a text came from, for the page ("SPDX License List v3.29.0").
    pub(crate) fn source(&self, id: &str) -> String {
        match self.index.get(id) {
            Some((_, s)) if s.starts_with("spdx/license-list-data ") => {
                format!(
                    "SPDX License List {}",
                    &s["spdx/license-list-data ".len()..]
                )
            }
            Some((_, s)) => s.clone(),
            None => "test text".to_string(),
        }
    }
}

fn toml_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

fn agent() -> ureq::Agent {
    let provider = rustls::crypto::aws_lc_rs::default_provider();
    let _ = rustls::crypto::CryptoProvider::install_default(provider);
    ureq::Agent::config_builder()
        .http_status_as_error(false)
        .timeout_global(Some(Duration::from_secs(20)))
        .build()
        .into()
}

fn fetch_spdx(agent: &ureq::Agent, id: &str, exception: bool) -> Result<(String, String), String> {
    let (dir, text_key) = if exception {
        ("exceptions", "licenseExceptionText")
    } else {
        ("details", "licenseText")
    };
    let url = format!("{SPDX_RAW}/{SPDX_TAG}/json/{dir}/{id}.json");
    let mut response = agent
        .get(&url)
        .call()
        .map_err(|e| format!("fetch {url}: {e}"))?;
    if response.status() != 200 {
        return Err(format!(
            "fetch {url}: HTTP {} (is {id:?} a valid SPDX id?)",
            response.status()
        ));
    }
    let doc: serde_json::Value = response
        .body_mut()
        .read_json()
        .map_err(|e| format!("parse {url}: {e}"))?;
    let name = doc["name"].as_str().unwrap_or(id).to_string();
    let body = doc[text_key]
        .as_str()
        .ok_or_else(|| format!("{url}: no {text_key}"))?
        .to_string();
    println!("licenses: fetched {id} ({name}) from spdx/license-list-data {SPDX_TAG}");
    Ok((name, body))
}

// ---- matching a package's file against the library ------------------------

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Match {
    /// The file is these canonical texts, apart from copyright lines.
    Canonical(Vec<String>),
    /// The file differs: print it as a variant of `primary`. `close`
    /// when it holds most of that licence's canonical text (then it
    /// replaces the canonical link in the row; otherwise it is an extra
    /// text, such as a third-party notices file).
    Variant { primary: String, close: bool },
    /// Too short to be a licence text (a pointer or a one-word name).
    Pointer,
}

/// Match a licence file. `declared` (the package's SPDX ids, exceptions
/// included) are tried first; a file that fits none of them is tried
/// against every text (packages that declare nothing, or the wrong id).
pub(crate) fn match_file(lib: &Library, text: &str, declared: &BTreeSet<String>) -> Match {
    let words = normalise(text);
    if words.len() < MIN_WORDS {
        return Match::Pointer;
    }
    let file = grams(&words);
    let pool = |all: bool| -> Vec<(&String, &Text)> {
        lib.texts
            .iter()
            .filter(|(id, _)| all || declared.contains(*id))
            .filter(|(id, _)| !id.starts_with("LicenseRef-OS"))
            .collect()
    };
    for all in [false, true] {
        if let Some(found) = cover(&file, &pool(all)) {
            return Match::Canonical(found);
        }
    }
    let coverage = |text: &Text| {
        let hit = text.grams.iter().filter(|g| file.contains(g)).count();
        hit as f64 / text.grams.len().max(1) as f64
    };
    let best_cover = pool(true)
        .iter()
        .map(|(_, t)| coverage(t))
        .fold(0.0, f64::max);
    if words.len() < POINTER_WORDS && best_cover < 0.3 {
        return Match::Pointer;
    }
    let primary = best_overlap(&file, &pool(false))
        .or_else(|| best_overlap(&file, &pool(true)))
        .unwrap_or_else(|| "LicenseRef-Unknown".to_string());
    let close = lib.texts.get(&primary).is_some_and(|t| coverage(t) >= 0.6);
    Match::Variant { primary, close }
}

/// Greedy cover of the file's trigrams by canonical texts: one text
/// for a plain LICENSE, two for a file that concatenates MIT and
/// Apache-2.0 (or Apache-2.0 plus its LLVM exception).
fn cover(file: &HashSet<u64>, pool: &[(&String, &Text)]) -> Option<Vec<String>> {
    let mut covered: HashSet<u64> = HashSet::new();
    let mut chosen: Vec<(&String, &Text)> = Vec::new();
    loop {
        let mut best: Option<(f64, usize)> = None;
        for (i, (_, text)) in pool.iter().enumerate() {
            if chosen.iter().any(|(_, c)| std::ptr::eq(*c, *text)) {
                continue;
            }
            let fresh = text
                .grams
                .iter()
                .filter(|g| file.contains(g) && !covered.contains(g))
                .count();
            let extra = text.grams.iter().filter(|g| !file.contains(g)).count();
            let score = fresh as f64 - 0.25 * extra as f64;
            if best.is_none_or(|(s, _)| score > s) {
                best = Some((score, i));
            }
        }
        match best {
            Some((score, i)) if score > 0.05 * file.len() as f64 => {
                let (_, text) = pool[i];
                covered.extend(text.grams.iter().filter(|g| file.contains(g)));
                chosen.push(pool[i]);
            }
            _ => break,
        }
    }
    let contained = covered.len() as f64 / file.len().max(1) as f64;
    let each_whole = chosen.iter().all(|(_, text)| {
        let hit = text.grams.iter().filter(|g| file.contains(g)).count();
        hit as f64 / text.grams.len().max(1) as f64 >= 0.6
    });
    if chosen.is_empty() || contained < 0.9 || !each_whole {
        return None;
    }
    let mut ids: Vec<String> = chosen.iter().map(|(id, _)| (*id).clone()).collect();
    ids.sort();
    Some(ids)
}

fn best_overlap(file: &HashSet<u64>, pool: &[(&String, &Text)]) -> Option<String> {
    pool.iter()
        .map(|(id, text)| (text.grams.iter().filter(|g| file.contains(g)).count(), *id))
        .filter(|(hits, _)| *hits > 0)
        .max_by(|a, b| a.0.cmp(&b.0).then_with(|| b.1.cmp(a.1)))
        .map(|(_, id)| id.clone())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const MIT: &str = "MIT License

Copyright (c) <year> <copyright holders>

Permission is hereby granted, free of charge, to any person obtaining a copy of this software and
associated documentation files (the \"Software\"), to deal in the Software without restriction, including
without limitation the rights to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is furnished to do so, subject to the
following conditions:

The above copyright notice and this permission notice shall be included in all copies or substantial
portions of the Software.

THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR IMPLIED, INCLUDING BUT NOT
LIMITED TO THE WARRANTIES OF MERCHANTABILITY, FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO
EVENT SHALL THE AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER LIABILITY, WHETHER
IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM, OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE
USE OR OTHER DEALINGS IN THE SOFTWARE.
";

    pub(crate) const ISC: &str = "ISC License

Copyright (c) <year> <copyright holders>

Permission to use, copy, modify, and/or distribute this software for any purpose with or without fee is
hereby granted, provided that the above copyright notice and this permission notice appear in all copies.

THE SOFTWARE IS PROVIDED \"AS IS\" AND THE AUTHOR DISCLAIMS ALL WARRANTIES WITH REGARD TO THIS SOFTWARE
INCLUDING ALL IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS. IN NO EVENT SHALL THE AUTHOR BE LIABLE
FOR ANY SPECIAL, DIRECT, INDIRECT, OR CONSEQUENTIAL DAMAGES OR ANY DAMAGES WHATSOEVER RESULTING FROM LOSS
OF USE, DATA OR PROFITS, WHETHER IN AN ACTION OF CONTRACT, NEGLIGENCE OR OTHER TORTIOUS ACTION, ARISING
OUT OF OR IN CONNECTION WITH THE USE OR PERFORMANCE OF THIS SOFTWARE.
";

    pub(crate) fn library(entries: &[(&str, &str)]) -> Library {
        let mut lib = Library {
            dir: PathBuf::new(),
            texts: BTreeMap::new(),
            index: BTreeMap::new(),
            fetched: Vec::new(),
        };
        for (id, body) in entries {
            lib.add(id, format!("{id} name"), body.to_string());
        }
        lib
    }

    fn set(ids: &[&str]) -> BTreeSet<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn expressions_tokenise_with_legacy_slash_and_exceptions() {
        let (l, e) = ids("(MIT/Apache-2.0) AND Apache-2.0 WITH LLVM-exception");
        assert_eq!(l, set(&["Apache-2.0", "MIT"]));
        assert_eq!(e, set(&["LLVM-exception"]));
        assert_eq!(tokens("MIT or Zlib")[1], Tok::Op("OR"));
    }

    #[test]
    fn copyright_lines_keep_notices_and_drop_templates_and_prose() {
        let text = "Copyright (c) <year> <copyright holders>\n\
                    Copyright 2016 Alex Crichton\n\
                    // Copyright The Rust Project Developers\n\
                    copyright notice and this permission notice shall be included\n\
                    COPYRIGHT HOLDERS AND CONTRIBUTORS\n\
                    Copyright Holder. This restriction only applies\n\
                    Copyright Holder(s) and the Author(s)\n\
                    © Jane Doe\n\
                    Copyright [yyyy] [name of copyright owner]\n";
        assert_eq!(
            copyright_lines(text),
            vec![
                "Copyright 2016 Alex Crichton",
                "Copyright The Rust Project Developers",
                "© Jane Doe"
            ]
        );
    }

    #[test]
    fn same_licence_under_two_holders_matches_one_canonical_text() {
        let lib = library(&[("MIT", MIT), ("ISC", ISC)]);
        let a = MIT.replace("<year> <copyright holders>", "2019 Alice");
        let b = MIT
            .replace(
                "Copyright (c) <year> <copyright holders>",
                "Copyright 2024 Bob\nAll rights reserved.",
            )
            .replace("MIT License\n", "The MIT License (MIT)\n");
        for text in [a, b] {
            assert_eq!(
                match_file(&lib, &text, &set(&["MIT"])),
                Match::Canonical(vec!["MIT".to_string()])
            );
        }
        assert_eq!(normalise(&MIT.replace("<year>", "2001")), normalise(MIT));
    }

    #[test]
    fn concatenated_texts_match_both_ids() {
        let lib = library(&[("MIT", MIT), ("ISC", ISC)]);
        let both = format!("{MIT}\n\n{ISC}");
        assert_eq!(
            match_file(&lib, &both, &set(&["ISC", "MIT"])),
            Match::Canonical(vec!["ISC".to_string(), "MIT".to_string()])
        );
    }

    #[test]
    fn wrong_or_missing_declaration_falls_back_to_the_whole_library() {
        let lib = library(&[("MIT", MIT), ("ISC", ISC)]);
        assert_eq!(
            match_file(&lib, ISC, &BTreeSet::new()),
            Match::Canonical(vec!["ISC".to_string()])
        );
    }

    #[test]
    fn extra_clauses_make_a_variant_and_pointers_are_ignored() {
        let lib = library(&[("MIT", MIT), ("ISC", ISC)]);
        let extended = format!(
            "{ISC}\nNote that this software also includes code under the following terms, \
             which the authors of this crate do not grant: the code in src/aes may only be \
             used for building cryptographic primitives that are audited annually by an \
             independent third party appointed by the original authors, and any binary \
             distribution must carry a printed notice to that effect on its packaging. \
             These terms survive any termination of the licence above and bind successors."
        );
        assert_eq!(
            match_file(&lib, &extended, &set(&["ISC"])),
            Match::Variant {
                primary: "ISC".to_string(),
                close: true
            }
        );
        assert_eq!(match_file(&lib, "MIT", &set(&["MIT"])), Match::Pointer);
        let pointer = "This project is dual-licensed under the Unlicense and MIT licenses. \
                       You may use this code under the terms of either license, see the \
                       LICENSE-MIT and UNLICENSE files in this directory.";
        assert_eq!(match_file(&lib, pointer, &set(&["MIT"])), Match::Pointer);
    }

    #[test]
    fn midline_and_wrapped_notices_are_cut_from_the_text() {
        let text = "This work is derived in part from the foo crate, Copyright (c)\n\
                    2015 The Rust Project Developers\n\
                    \n\
                    Copyright 2011-2017 Google Inc.\n\
                    \x20         2013 Jack Lloyd\n\
                    Body text follows here.\n\
                    (c) You must retain all notices.\n";
        assert_eq!(
            copyright_lines(text),
            vec![
                "Copyright (c) 2015 The Rust Project Developers",
                "Copyright 2011-2017 Google Inc.",
                "Copyright 2013 Jack Lloyd",
            ]
        );
        assert_eq!(
            normalise(text).join(" "),
            "this work is derived in part from the foo crate body text follows here \
             c you must retain all notices"
        );
    }

    #[test]
    fn variants_print_without_copyright_lines() {
        let text = "Copyright 2020 X\nAll rights reserved.\n\n\n\nBody line\n\n\n\nEnd\n";
        assert_eq!(without_copyrights(text), "Body line\n\nEnd");
    }

    #[test]
    fn year_detection_needs_four_isolated_digits() {
        assert!(has_year("Copyright 2019-2024 X"));
        assert!(!has_year("Copyright 120199 X"));
        assert!(!has_year("version 1.2.3"));
    }
}
