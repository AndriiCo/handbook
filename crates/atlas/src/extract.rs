//! Multimodal extraction: fetched bytes → model-ready documents.
//!
//! Text-like kinds extract locally and deterministically. Binary
//! kinds (`Pdf`, `Audio`, `Image`) default to API-native
//! understanding (the server parses PDFs to text + per-page images;
//! audio rides the transcribe endpoint or `input_audio`), with an
//! external-tool fallback for PDFs when the operator configures one.

use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

use crate::config::SourceKind;
use crate::error::{Error, Result};
use crate::source::FetchedSource;

/// How PDFs become model input.
#[derive(Debug, Clone, Default)]
pub enum PdfMode {
    /// Send bytes to the model (`input_file` / Files API). The
    /// server parses text + per-page images (≤50 pages full).
    #[default]
    ApiNative,
    /// Pipe through an external tool (`pdftotext -layout - -` by
    /// default). No shell: argv is executed directly.
    External {
        argv: Vec<String>,
        timeout: Duration,
        max_output: u64,
    },
}

/// Extraction knobs (built from pack + CLI).
#[derive(Debug, Clone, Default)]
pub struct ExtractConfig {
    pub pdf: PdfMode,
}

/// One extracted page/span with byte offsets into [`Document::text`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Page {
    pub index: usize,
    pub byte_start: usize,
    pub byte_end: usize,
}

/// A binary payload for model-native understanding.
#[derive(Debug, Clone)]
pub enum NativePayload {
    Pdf(Vec<u8>),
    Audio {
        bytes: Vec<u8>,
        content_type: String,
    },
    Image {
        bytes: Vec<u8>,
        content_type: String,
    },
}

/// One model-ready document with provenance.
#[derive(Debug, Clone)]
pub struct Document {
    /// Source name (pack `[[sources]]`).
    pub id: String,
    pub kind: SourceKind,
    /// Locally extracted text (empty for API-native binaries).
    pub text: String,
    pub pages: Vec<Page>,
    /// sha256 of the fetched bytes this derives from.
    pub derived_from: String,
    /// Binary payload when the model reads the source natively.
    pub native: Option<NativePayload>,
}

/// Extract one fetched source.
pub fn extract(fetched: &FetchedSource, cfg: &ExtractConfig) -> Result<Document> {
    let id = fetched.name.clone();
    let derived_from = fetched.sha256.clone();
    let (kind, text, native) = match fetched.spec.kind {
        SourceKind::Text => (SourceKind::Text, lossy(&fetched.bytes), None),
        SourceKind::Html => (SourceKind::Html, html_to_text(&fetched.bytes), None),
        SourceKind::Svg => (SourceKind::Svg, svg_to_text(&fetched.bytes), None),
        SourceKind::Pdf => match &cfg.pdf {
            PdfMode::ApiNative => (
                SourceKind::Pdf,
                String::new(),
                Some(NativePayload::Pdf(fetched.bytes.clone())),
            ),
            PdfMode::External {
                argv,
                timeout,
                max_output,
            } => (
                SourceKind::Pdf,
                run_external(argv, &fetched.bytes, *timeout, *max_output)
                    .map_err(|e| Error::Extract(format!("doc '{id}': pdf tool failed: {e}")))?,
                None,
            ),
        },
        SourceKind::Audio => (
            SourceKind::Audio,
            String::new(),
            Some(NativePayload::Audio {
                bytes: fetched.bytes.clone(),
                content_type: sniff_audio(&fetched.bytes).to_string(),
            }),
        ),
        SourceKind::Image => (
            SourceKind::Image,
            String::new(),
            Some(NativePayload::Image {
                bytes: fetched.bytes.clone(),
                content_type: sniff_image(&fetched.bytes).to_string(),
            }),
        ),
    };
    let pages = if text.is_empty() {
        Vec::new()
    } else {
        vec![Page {
            index: 0,
            byte_start: 0,
            byte_end: text.len(),
        }]
    };
    Ok(Document {
        id,
        kind,
        text,
        pages,
        derived_from,
        native,
    })
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// HTML → readable text. Tags are stripped; tables keep `|` rows so
/// the model still sees the grid; entities are decoded.
pub fn html_to_text(bytes: &[u8]) -> String {
    let html = lossy(bytes);
    let mut out = String::with_capacity(html.len());
    let mut in_tag = false;
    let mut cells_in_row = 0usize;
    let mut pending_space = false;
    let chars: Vec<char> = html.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if c == '<' {
            let tag: String = chars[i..].iter().take_while(|c| **c != '>').collect();
            let tag = tag.to_lowercase();
            let is_close = tag.starts_with("</");
            let name: String = tag
                .trim_start_matches("</")
                .trim_start_matches('<')
                .chars()
                .take_while(|c| c.is_ascii_alphanumeric())
                .collect();
            if matches!(
                name.as_str(),
                "p" | "br" | "div" | "tr" | "li" | "h1" | "h2" | "h3" | "h4" | "table"
            ) {
                out.push('\n');
            }
            if name == "tr" && !is_close {
                cells_in_row = 0;
            }
            if (name == "td" || name == "th") && !is_close {
                if cells_in_row > 0 {
                    out.push_str(" | ");
                }
                cells_in_row += 1;
            }
            in_tag = true;
            i += 1;
            continue;
        }
        if c == '>' {
            in_tag = false;
            i += 1;
            continue;
        }
        if in_tag {
            i += 1;
            continue;
        }
        if c.is_whitespace() {
            pending_space = true;
        } else {
            if pending_space && !out.ends_with(['\n', ' ', '|']) {
                out.push(' ');
            }
            pending_space = false;
            out.push(c);
        }
        i += 1;
    }
    decode_entities(&out)
        .lines()
        .map(str::trim_end)
        .filter(|line| !line.trim().is_empty())
        .collect::<Vec<_>>()
        .join("\n")
}

fn decode_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&nbsp;", " ")
}

/// SVG → structured text: `<text>` nodes verbatim (they carry the
/// labels on status maps) plus an element inventory for shape.
pub fn svg_to_text(bytes: &[u8]) -> String {
    let svg = lossy(bytes);
    let mut texts = Vec::new();
    let mut counts: std::collections::BTreeMap<String, usize> = Default::default();
    let mut rest = svg.as_str();
    while let Some(start) = rest.find('<') {
        rest = &rest[start + 1..];
        if rest.starts_with('/') || rest.starts_with('!') || rest.starts_with('?') {
            continue;
        }
        let name: String = rest
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect();
        if name.is_empty() {
            continue;
        }
        *counts.entry(name.clone()).or_insert(0) += 1;
        if name == "text" {
            let Some(body_start) = rest.find('>') else {
                break;
            };
            let body = &rest[body_start + 1..];
            let Some(end) = body.find("</text>") else {
                break;
            };
            let node = body[..end].trim();
            if !node.is_empty() {
                // Nested tspan tags: strip inner markup.
                let mut plain = String::new();
                let mut in_tag = false;
                for c in node.chars() {
                    if c == '<' {
                        in_tag = true;
                    } else if c == '>' {
                        in_tag = false;
                    } else if !in_tag {
                        plain.push(c);
                    }
                }
                let plain = plain.trim();
                if !plain.is_empty() {
                    texts.push(plain.to_string());
                }
            }
            rest = &body[end..];
        }
    }
    let mut out = String::from("[svg labels]\n");
    for text in &texts {
        out.push_str(text);
        out.push('\n');
    }
    out.push_str("[svg shapes]\n");
    for (name, count) in &counts {
        out.push_str(&format!("{name}: {count}\n"));
    }
    out
}

fn sniff_audio(bytes: &[u8]) -> &'static str {
    if bytes.len() > 12 && &bytes[4..8] == b"ftyp" {
        "audio/mp4"
    } else if bytes.starts_with(b"ID3")
        || (bytes.len() > 2 && bytes[0] == 0xFF && bytes[1] & 0xE0 == 0xE0)
    {
        "audio/mpeg"
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        "audio/wav"
    } else if bytes.starts_with(b"OggS") {
        "audio/ogg"
    } else {
        "application/octet-stream"
    }
}

fn sniff_image(bytes: &[u8]) -> &'static str {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else if bytes.starts_with(b"GIF8") {
        "image/gif"
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        "image/webp"
    } else {
        "application/octet-stream"
    }
}

/// Run an external extractor: bytes on stdin, text on stdout. No
/// shell — `argv[0]` is executed directly with piped stdio.
fn run_external(
    argv: &[String],
    input: &[u8],
    timeout: Duration,
    max_output: u64,
) -> Result<String> {
    let (program, args) = argv
        .split_first()
        .ok_or_else(|| Error::Extract("pdf tool argv is empty".to_string()))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir(PathBuf::from("."))
        .spawn()
        .map_err(|e| Error::Extract(format!("spawn {program}: {e}")))?;
    if let Some(stdin) = child.stdin.take() {
        use std::io::Write;
        let mut stdin = stdin;
        stdin.write_all(input).map_err(Error::Io)?;
    }
    let (tx, rx) = std::sync::mpsc::channel();
    let stdout = child.stdout.take();
    std::thread::spawn(move || {
        let mut capped = Vec::new();
        if let Some(stdout) = stdout {
            use std::io::Read;
            let mut reader = std::io::BufReader::new(stdout).take(max_output + 1);
            let _ = reader.read_to_end(&mut capped);
        }
        let _ = tx.send(capped);
    });
    let output = rx.recv_timeout(timeout).map_err(|_| {
        let _ = child.kill();
        Error::Extract(format!("pdf tool timed out after {timeout:?}"))
    })?;
    let _ = child.wait();
    if output.len() as u64 > max_output {
        return Err(Error::Extract(format!(
            "pdf tool output exceeds {max_output} bytes"
        )));
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn html_tables_keep_rows() {
        let html = b"<table><tr><th>State</th><th>EIF</th></tr>\
            <tr><td>Viet Nam</td><td>2026-09-11</td></tr></table><p>Note&nbsp;1 &amp; 2</p>";
        let text = html_to_text(html);
        assert!(text.contains("State | EIF"), "{text}");
        assert!(text.contains("Viet Nam | 2026-09-11"), "{text}");
        assert!(text.contains("Note 1 & 2"), "{text}");
    }

    #[test]
    fn svg_labels_and_shapes() {
        let svg = br#"<svg><text x="1">Viet Nam</text><g><text>Member</text></g>
            <rect width="4"/><rect width="5"/><path d="M0 0"/></svg>"#;
        let text = svg_to_text(svg);
        assert!(text.contains("Viet Nam"), "{text}");
        assert!(text.contains("Member"), "{text}");
        assert!(text.contains("rect: 2"), "{text}");
        assert!(text.contains("text: 2"), "{text}");
    }

    #[test]
    fn sniffers_name_common_types() {
        assert_eq!(sniff_audio(b"ID3blah"), "audio/mpeg");
        assert_eq!(sniff_audio(b"\x89PNG?no"), "application/octet-stream");
        assert_eq!(sniff_image(&[0x89, b'P', b'N', b'G', 0]), "image/png");
        assert_eq!(sniff_image(b"GIF89a"), "image/gif");
    }

    #[test]
    fn external_missing_tool_errors() {
        let err = run_external(
            &["atlas-no-such-tool-xyz".to_string()],
            b"in",
            Duration::from_secs(5),
            1024,
        )
        .unwrap_err();
        assert!(format!("{err}").contains("spawn"), "{err}");
    }
}
