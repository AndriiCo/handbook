//! Submission renderers: printpdf server PDF + HTML-print seam.
//!
//! [`FormRenderer`] is the P2-R6 seam: the PDF artifact and the future
//! HTML print view share one field-layout source (the template +
//! validated values). v1 ships [`PdfRenderer`] (wet-ink-only); the
//! ultimate goal is HTML-print-first with PDF on request.
//!
//! PDF determinism (P2-R5): printpdf 0.12.8 embeds two random trailer
//! `/ID` strings per save, so the renderer normalizes them to zeros
//! after saving. Metadata dates stay at the printpdf default (epoch);
//! the generation time is recorded in the header block and alongside
//! the artifact, never embedded variably.

use std::collections::BTreeMap;
use std::fmt;

use serde_json::Value;
use sha2::{Digest, Sha256};

use crate::template::{FormTemplate, SignatureMode};

/// US Letter in mm (US government forms print on Letter, not A4).
const PAGE_W_MM: f32 = 215.9;
const PAGE_H_MM: f32 = 279.4;

/// One validated fill-in ready to render.
pub struct Submission<'a> {
    pub template: &'a FormTemplate,
    pub values: &'a BTreeMap<String, Value>,
    /// Generation time, unix seconds; printed in the header block.
    pub generated_at_unix: i64,
    /// Handbook build the prefilled facts came from, if known.
    pub build_no: Option<i64>,
    /// Replica-acceptance notice for the header, if any.
    pub replica_notice: Option<&'a str>,
}

/// Rendered bytes plus audit metadata (P2-R4).
#[derive(Debug, Clone)]
pub struct RenderArtifact {
    pub bytes: Vec<u8>,
    pub sha256_hex: String,
    pub content_type: &'static str,
}

/// One field-layout source, many outputs (P2-R6).
pub trait FormRenderer {
    fn content_type(&self) -> &'static str;
    /// Render a validated submission.
    ///
    /// # Errors
    ///
    /// Returns [`RenderError`] when the template asks for something
    /// the renderer cannot do (drawn signatures in P2).
    fn render(&self, submission: &Submission) -> Result<RenderArtifact, RenderError>;
}

/// [`FormRenderer`] failure.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum RenderError {
    DrawnSignaturesUnsupported,
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RenderError::DrawnSignaturesUnsupported => {
                write!(f, "drawn signatures are not rendered in P2 (wet-ink only)")
            }
        }
    }
}

impl std::error::Error for RenderError {}

/// Server-rendered filled PDF via printpdf (P2-R1, P2-R2).
///
/// Clean-room redraw: our boxes/lines/text at the template's measured
/// rects, never an overlay on the official PDF (P2-R4). Wet-ink only:
/// the signature block is an empty signature + date line.
#[derive(Debug, Clone, Copy, Default)]
pub struct PdfRenderer {
    /// Compress content streams (default true). Uncompressed output is
    /// only useful to tests that grep for header text.
    pub optimize: bool,
}

impl PdfRenderer {
    pub fn new() -> Self {
        Self { optimize: true }
    }

    /// Test hook: uncompressed streams so assertions can find text.
    pub fn unoptimized() -> Self {
        Self { optimize: false }
    }
}

impl FormRenderer for PdfRenderer {
    fn content_type(&self) -> &'static str {
        "application/pdf"
    }

    fn render(&self, submission: &Submission) -> Result<RenderArtifact, RenderError> {
        use printpdf::{Line, LinePoint, Mm, Op, PdfDocument, PdfPage, PdfSaveOptions, Point};

        let template = submission.template;
        if template.signature_mode != SignatureMode::WetInk {
            return Err(RenderError::DrawnSignaturesUnsupported);
        }

        let max_page = template
            .fields
            .iter()
            .map(|f| f.pdf.page.max(1))
            .max()
            .unwrap_or(1);
        let mut pages = Vec::with_capacity(max_page as usize);
        for page_no in 1..=max_page {
            let mut ops = Vec::new();
            if page_no == 1 {
                push_header(&mut ops, template, submission);
            }
            for field in template
                .fields
                .iter()
                .filter(|f| f.pdf.page.max(1) == page_no)
            {
                let value = display_value(submission.values.get(&field.key));
                push_text_line(
                    &mut ops,
                    &format!("{}: {value}", field.label),
                    field.pdf.x_mm,
                    field.pdf.y_mm,
                    field.pdf.font_size_pt,
                );
                // Underline the value area: the replica's writing line.
                let y = PAGE_H_MM - field.pdf.y_mm - 2.0;
                ops.push(Op::DrawLine {
                    line: Line {
                        points: vec![
                            LinePoint {
                                p: Point::new(Mm(field.pdf.x_mm), Mm(y)),
                                bezier: false,
                            },
                            LinePoint {
                                p: Point::new(Mm(field.pdf.x_mm + field.pdf.w_mm), Mm(y)),
                                bezier: false,
                            },
                        ],
                        is_closed: false,
                    },
                });
            }
            if page_no == max_page {
                // Wet-ink block: empty lines, signed in pen after printing.
                // The line sits 30mm above the page foot (PDF origin);
                // labels sit 8mm above the line.
                let y_sig = 30.0;
                push_text_line(&mut ops, "Signature:", 20.0, PAGE_H_MM - y_sig - 8.0, 11.0);
                ops.push(Op::DrawLine {
                    line: Line {
                        points: vec![
                            LinePoint {
                                p: Point::new(Mm(55.0), Mm(y_sig)),
                                bezier: false,
                            },
                            LinePoint {
                                p: Point::new(Mm(140.0), Mm(y_sig)),
                                bezier: false,
                            },
                        ],
                        is_closed: false,
                    },
                });
                push_text_line(&mut ops, "Date:", 150.0, PAGE_H_MM - y_sig - 8.0, 11.0);
                ops.push(Op::DrawLine {
                    line: Line {
                        points: vec![
                            LinePoint {
                                p: Point::new(Mm(165.0), Mm(y_sig)),
                                bezier: false,
                            },
                            LinePoint {
                                p: Point::new(Mm(195.0), Mm(y_sig)),
                                bezier: false,
                            },
                        ],
                        is_closed: false,
                    },
                });
            }
            pages.push(PdfPage::new(Mm(PAGE_W_MM), Mm(PAGE_H_MM), ops));
        }

        let title = format!("{} v{}", template.code, template.version);
        let mut doc = PdfDocument::new(&title);
        let mut warnings = Vec::new();
        let mut bytes = doc.with_pages(pages).save(
            &PdfSaveOptions {
                optimize: self.optimize,
                ..Default::default()
            },
            &mut warnings,
        );
        normalize_trailer_ids(&mut bytes);
        let sha256_hex = format!("{:x}", Sha256::digest(&bytes));
        Ok(RenderArtifact {
            bytes,
            sha256_hex,
            content_type: self.content_type(),
        })
    }
}

fn push_text_line(
    ops: &mut Vec<printpdf::Op>,
    text: &str,
    x_mm: f32,
    y_from_top_mm: f32,
    size_pt: f32,
) {
    use printpdf::{BuiltinFont, Mm, Op, PdfFontHandle, Point, Pt, TextItem};

    ops.push(Op::SaveGraphicsState);
    ops.push(Op::StartTextSection);
    ops.push(Op::SetTextCursor {
        pos: Point::new(Mm(x_mm), Mm(PAGE_H_MM - y_from_top_mm)),
    });
    ops.push(Op::SetFont {
        font: PdfFontHandle::Builtin(BuiltinFont::Helvetica),
        size: Pt(size_pt),
    });
    ops.push(Op::SetLineHeight { lh: Pt(size_pt) });
    ops.push(Op::ShowText {
        items: vec![TextItem::Text(sanitize(text))],
    });
    ops.push(Op::EndTextSection);
    ops.push(Op::RestoreGraphicsState);
}

fn push_header(ops: &mut Vec<printpdf::Op>, template: &FormTemplate, submission: &Submission) {
    use printpdf::{BuiltinFont, Mm, Op, PdfFontHandle, Point, Pt, TextItem};

    let mut lines = vec![
        template.title.clone(),
        format!(
            "{} v{} ({})",
            template.code, template.version, template.authority_revision
        ),
        match submission.build_no {
            Some(build) => format!(
                "generated {} from accepted facts as of build #{build}",
                format_unix_utc(submission.generated_at_unix)
            ),
            None => format!(
                "generated {}",
                format_unix_utc(submission.generated_at_unix)
            ),
        },
    ];
    if let Some(notice) = submission.replica_notice {
        lines.push(notice.to_string());
    }
    for (i, text) in lines.iter().enumerate() {
        let y = 12.0 + i as f32 * 6.0;
        ops.push(Op::SaveGraphicsState);
        ops.push(Op::StartTextSection);
        ops.push(Op::SetTextCursor {
            pos: Point::new(Mm(20.0), Mm(PAGE_H_MM - y)),
        });
        ops.push(Op::SetFont {
            font: PdfFontHandle::Builtin(if i == 0 {
                BuiltinFont::HelveticaBold
            } else {
                BuiltinFont::Helvetica
            }),
            size: Pt(if i == 0 { 14.0 } else { 9.0 }),
        });
        ops.push(Op::SetLineHeight { lh: Pt(10.0) });
        ops.push(Op::ShowText {
            items: vec![TextItem::Text(sanitize(text))],
        });
        ops.push(Op::EndTextSection);
        ops.push(Op::RestoreGraphicsState);
    }
}

/// P2-R5: printpdf writes two random 32-char trailer `/ID` strings per
/// save. Zero the two parenthesized groups after `/ID` so the same
/// submission renders byte-identical PDFs.
fn normalize_trailer_ids(bytes: &mut [u8]) {
    let marker = b"/ID";
    let Some(start) = bytes
        .windows(marker.len())
        .position(|w| w == marker)
        .map(|p| p + marker.len())
    else {
        return;
    };
    let mut i = start;
    for _ in 0..2 {
        while i < bytes.len() && bytes[i] != b'(' {
            i += 1;
        }
        if i >= bytes.len() {
            return;
        }
        i += 1;
        let group_start = i;
        while i < bytes.len() && bytes[i] != b')' {
            i += 1;
        }
        for byte in &mut bytes[group_start..i] {
            *byte = b'0';
        }
    }
}

fn display_value(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Bool(b)) => {
            if *b {
                "yes".to_string()
            } else {
                "no".to_string()
            }
        }
        Some(Value::Number(n)) => n.to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| display_value(Some(v)))
            .collect::<Vec<_>>()
            .join(", "),
        Some(Value::Object(map)) => map
            .iter()
            .map(|(k, v)| format!("{k}: {}", display_value(Some(v))))
            .collect::<Vec<_>>()
            .join("; "),
    }
}

/// Strip control characters (keep tab/newline as spaces): PDF text
/// sections must not carry raw control bytes.
fn sanitize(text: &str) -> String {
    text.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect()
}

/// Unix seconds → `YYYY-MM-DD HH:MM:SS UTC` (no date crate needed).
fn format_unix_utc(unix: i64) -> String {
    let days = unix.div_euclid(86_400);
    let secs = unix.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days + 719_468);
    format!(
        "{year:04}-{month:02}-{day:02} {:02}:{:02}:{:02} UTC",
        secs / 3600,
        secs % 3600 / 60,
        secs % 60
    )
}

/// Howard Hinnant's civil-from-days (days since 0000-03-01… shifted by
/// 719468 from unix epoch). Clamped to non-negative output.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// HTML-print-first renderer (P2-R6 roadmap seam).
///
/// Emits a standalone printable page at US Letter size: the same header
/// block, field list, and wet-ink signature line as the PDF, styled by
/// an embedded print stylesheet. Values are HTML-escaped.
#[derive(Debug, Clone, Copy, Default)]
pub struct HtmlPrintRenderer;

impl FormRenderer for HtmlPrintRenderer {
    fn content_type(&self) -> &'static str {
        "text/html; charset=utf-8"
    }

    fn render(&self, submission: &Submission) -> Result<RenderArtifact, RenderError> {
        let template = submission.template;
        if template.signature_mode != SignatureMode::WetInk {
            return Err(RenderError::DrawnSignaturesUnsupported);
        }
        let mut rows = String::new();
        for field in &template.fields {
            rows.push_str(&format!(
                "<tr><th>{}</th><td>{}</td></tr>\n",
                escape(&field.label),
                escape(&display_value(submission.values.get(&field.key)))
            ));
        }
        let build = submission
            .build_no
            .map(|b| format!(" from accepted facts as of build #{b}"))
            .unwrap_or_default();
        let notice = submission
            .replica_notice
            .map(|n| format!("<p class=\"notice\">{}</p>\n", escape(n)))
            .unwrap_or_default();
        let html = format!(
            "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n\
             <title>{title}</title>\n<style>\n\
             @page {{ size: Letter; margin: 18mm 15mm; }}\n\
             body {{ font-family: sans-serif; color: #111; }}\n\
             table {{ border-collapse: collapse; width: 100%; }}\n\
             th, td {{ border-bottom: 1px solid #999; padding: 4px 6px; \
             text-align: left; vertical-align: top; }}\n\
             .notice {{ border: 1px solid #333; padding: 8px; }}\n\
             .sig {{ margin-top: 2em; display: flex; gap: 2em; }}\n\
             .sig span {{ border-top: 1px solid #000; padding-top: 2px; \
             min-width: 12em; }}\n\
             </style>\n</head>\n<body>\n<h1>{title}</h1>\n\
             <p>{code} v{version} ({revision})<br>\n\
             generated {generated}{build}</p>\n{notice}\
             <table>\n{rows}</table>\n\
             <div class=\"sig\"><span>Signature</span><span>Date</span></div>\n\
             </body>\n</html>\n",
            title = escape(&template.title),
            code = escape(&template.code),
            version = template.version,
            revision = escape(&template.authority_revision),
            generated = format_unix_utc(submission.generated_at_unix),
        );
        let sha256_hex = format!("{:x}", Sha256::digest(html.as_bytes()));
        Ok(RenderArtifact {
            bytes: html.into_bytes(),
            sha256_hex,
            content_type: self.content_type(),
        })
    }
}

fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    fn submission<'a>(
        template: &'a FormTemplate,
        values: &'a BTreeMap<String, Value>,
    ) -> Submission<'a> {
        Submission {
            template,
            values,
            generated_at_unix: 1_791_979_200,
            build_no: Some(7),
            replica_notice: None,
        }
    }

    #[test]
    fn pdf_renders_and_is_deterministic() {
        let template = crate::template::tests::sample_template();
        let values: BTreeMap<String, Value> = BTreeMap::from([(
            "full_name".to_string(),
            Value::String("Jane Doe".to_string()),
        )]);
        let renderer = PdfRenderer::new();
        let first = renderer.render(&submission(&template, &values)).unwrap();
        let second = renderer.render(&submission(&template, &values)).unwrap();
        assert_eq!(first.content_type, "application/pdf");
        assert!(first.bytes.starts_with(b"%PDF-"));
        assert_eq!(first.bytes, second.bytes);
        assert_eq!(first.sha256_hex.len(), 64);
        assert_eq!(
            first.sha256_hex,
            format!("{:x}", Sha256::digest(&first.bytes))
        );
    }

    #[test]
    fn pdf_header_is_present_uncompressed() {
        let template = crate::template::tests::sample_template();
        let values: BTreeMap<String, Value> = BTreeMap::from([(
            "full_name".to_string(),
            Value::String("Jane Doe".to_string()),
        )]);
        let bytes = PdfRenderer::unoptimized()
            .render(&submission(&template, &values))
            .unwrap()
            .bytes;
        let text = String::from_utf8_lossy(&bytes);
        // printpdf hex-encodes strings containing parens: this is
        // "wa-apostille-request v1 (Rev 06/2025)".
        assert!(
            text.contains(
                "77612D61706F7374696C6C652D7265717565737420763120285265762030362F3230323529"
            ),
            "header code + revision, got: {text}"
        );
        assert!(text.contains("build #7"), "build ref");
        assert!(text.contains("Jane Doe"), "value");
        assert!(text.contains("Signature:"), "wet-ink line");
    }

    #[test]
    fn pdf_rejects_drawn_mode() {
        let mut template = crate::template::tests::sample_template();
        template.signature_mode = SignatureMode::DrawnOptional;
        let values = BTreeMap::new();
        assert_eq!(
            PdfRenderer::new()
                .render(&submission(&template, &values))
                .unwrap_err(),
            RenderError::DrawnSignaturesUnsupported
        );
    }

    #[test]
    fn html_print_shares_layout_and_escapes() {
        let template = crate::template::tests::sample_template();
        let values: BTreeMap<String, Value> = BTreeMap::from([(
            "full_name".to_string(),
            Value::String("<script>alert(1)</script>".to_string()),
        )]);
        let artifact = HtmlPrintRenderer
            .render(&submission(&template, &values))
            .unwrap();
        let html = String::from_utf8(artifact.bytes).unwrap();
        assert!(html.contains("@page"), "print stylesheet");
        assert!(html.contains("size: Letter"), "letter size");
        assert!(html.contains("wa-apostille-request"), "header");
        assert!(html.contains("Signature"), "wet-ink line");
        assert!(!html.contains("<script>"), "values are escaped");
        assert!(html.contains("&lt;script&gt;"), "escaped value");
    }

    #[test]
    fn unix_formatting() {
        assert_eq!(format_unix_utc(0), "1970-01-01 00:00:00 UTC");
        assert_eq!(format_unix_utc(1_791_849_600), "2026-10-13 00:00:00 UTC");
        assert_eq!(format_unix_utc(1_582_934_400), "2020-02-29 00:00:00 UTC");
    }
}
