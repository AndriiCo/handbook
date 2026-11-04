//! RFC 2822 parsing for the mail sidecar: headers of interest + the
//! text a reader would see.
//!
//! Two layers. Trust-bearing headers (From, `Authentication-Results`,
//! threading ids) come from our own unfolded scan of the raw head, so
//! the sender rules in `poll` never ride on a third-party parser's
//! leniency. Everything a reader sees comes from `mail-parser`: nested
//! multiparts at any depth, every charset (`full_encoding`), RFC 2047
//! encoded subjects, inline text parts in order, forwarded
//! `message/rfc822` parts, text attachments, and the text layer of PDF
//! attachments (lopdf). Delivery reports are recognized here too
//! ([`crate::report`]). The raw bytes stay the capture blob, so a
//! better extractor can re-derive sidecars later without re-fetching.

use mail_parser::{Message, MessageParser, MimeHeaders, PartType};

use crate::report::{self, Report};

/// Largest PDF attachment read (bigger ones are noted, not parsed).
pub const MAX_PDF_BYTES: usize = 10 << 20;
/// Pages read per PDF.
pub const MAX_PDF_PAGES: usize = 50;
/// Characters kept per attachment or forwarded message.
pub const MAX_SECTION_CHARS: usize = 100_000;
/// Characters kept for the whole body.
pub const MAX_BODY_CHARS: usize = 300_000;
/// Forwarded-message nesting followed.
const MAX_DEPTH: usize = 3;

/// What the poller needs from one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMail {
    pub from: String,
    pub to: String,
    pub date: String,
    /// Decoded (RFC 2047) subject.
    pub subject: String,
    /// All `Authentication-Results` values, topmost first.
    pub auth_results: Vec<String>,
    /// Threading headers, raw (see [`message_ids`]).
    pub message_id: String,
    pub in_reply_to: String,
    pub references: String,
    /// Sidecar body: inline text parts (HTML tag-stripped when that is
    /// all there is), then forwarded messages and attachment text, each
    /// under a `[Attachment: name]` line.
    pub body_text: String,
    /// Every attachment seen, with how much text it gave.
    pub attachments: Vec<AttachmentNote>,
    /// Set when this message is a bounce or complaint.
    pub report: Option<Report>,
}

/// One attachment and what was read from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AttachmentNote {
    pub name: String,
    pub content_type: String,
    /// Characters of text taken into `body_text` (0 = none).
    pub chars: usize,
    /// Why nothing (or less) was read, when that happened.
    pub note: &'static str,
}

/// Parse headers + sidecar body from RFC 2822 bytes, reading PDF
/// attachments. Never fails: unparseable input yields empty fields
/// (the blob stays authoritative).
pub fn parse(raw: &[u8]) -> ParsedMail {
    parse_with(raw, true)
}

/// [`parse`], with PDF attachments read only when `read_pdfs` (the
/// poller turns it off for a message that killed a previous run).
pub fn parse_with(raw: &[u8], read_pdfs: bool) -> ParsedMail {
    let text = String::from_utf8_lossy(raw);
    let (head, _) = split_head_body(&text);
    let headers = header_fields(head);
    let get = |name: &str| {
        headers
            .iter()
            .find(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .unwrap_or_default()
    };
    let all = |name: &str| {
        headers
            .iter()
            .filter(|(n, _)| n == name)
            .map(|(_, v)| v.clone())
            .collect()
    };
    let message = MessageParser::default().parse(raw);
    let mut attachments = Vec::new();
    let (subject, body_text, report) = match &message {
        Some(message) => {
            let subject = message
                .subject()
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|| get("subject"));
            let body = clip(
                &message_text(message, read_pdfs, 0, &mut attachments),
                MAX_BODY_CHARS,
            );
            (
                subject,
                body,
                report::from_message(message, &get("x-failed-recipients")),
            )
        }
        None => (get("subject"), String::new(), None),
    };
    ParsedMail {
        from: get("from"),
        to: get("to"),
        date: get("date"),
        subject,
        auth_results: all("authentication-results"),
        message_id: get("message-id"),
        in_reply_to: get("in-reply-to"),
        references: get("references"),
        body_text,
        attachments,
        report,
    }
}

/// Inline bodies, then attachments (forwarded messages recurse).
fn message_text(
    message: &Message<'_>,
    read_pdfs: bool,
    depth: usize,
    notes: &mut Vec<AttachmentNote>,
) -> String {
    let mut sections: Vec<String> = Vec::new();
    for id in &message.text_body {
        let Some(part) = message.parts.get(*id as usize) else {
            continue;
        };
        let text = match &part.body {
            PartType::Text(text) => text.trim().to_string(),
            PartType::Html(html) => ingest::capture::extract_text(html.as_bytes()),
            _ => continue,
        };
        if !text.is_empty() {
            sections.push(text);
        }
    }
    for id in &message.attachments {
        let Some(part) = message.parts.get(*id as usize) else {
            continue;
        };
        let content_type = part
            .content_type()
            .map(|ct| {
                format!(
                    "{}/{}",
                    ct.ctype().to_ascii_lowercase(),
                    ct.subtype().unwrap_or("").to_ascii_lowercase()
                )
            })
            .unwrap_or_default();
        if content_type.starts_with("image/") {
            continue;
        }
        let name = part.attachment_name().unwrap_or("").trim().to_string();
        let (text, note) = match &part.body {
            PartType::Message(inner) if depth < MAX_DEPTH => {
                let subject = inner.subject().unwrap_or("").trim();
                let body = message_text(inner, read_pdfs, depth + 1, notes);
                (format!("Subject: {subject}\n\n{body}"), "")
            }
            PartType::Message(_) => (String::new(), "nested too deep"),
            PartType::Text(text) => (text.trim().to_string(), ""),
            PartType::Html(html) => (ingest::capture::extract_text(html.as_bytes()), ""),
            PartType::Binary(bytes) | PartType::InlineBinary(bytes)
                if content_type == "application/pdf"
                    || name.to_ascii_lowercase().ends_with(".pdf") =>
            {
                if !read_pdfs {
                    (String::new(), "skipped after a failed read")
                } else if bytes.len() > MAX_PDF_BYTES {
                    (String::new(), "too large to read")
                } else {
                    match pdf_text(bytes) {
                        Some(text) => (text, ""),
                        None => (String::new(), "no text layer"),
                    }
                }
            }
            _ => (String::new(), "not text"),
        };
        let text = clip(text.trim(), MAX_SECTION_CHARS);
        let label = if name.is_empty() {
            content_type.clone()
        } else {
            name.clone()
        };
        notes.push(AttachmentNote {
            name: label.clone(),
            content_type: content_type.clone(),
            chars: text.chars().count(),
            note,
        });
        if !text.is_empty() {
            sections.push(format!("[Attachment: {label}]\n{text}"));
        }
    }
    sections.join("\n\n")
}

/// Text layer of a PDF, page by page (pages that fail are skipped).
fn pdf_text(bytes: &[u8]) -> Option<String> {
    let doc = lopdf::Document::load_mem(bytes).ok()?;
    let mut out = String::new();
    for page in doc.get_pages().keys().copied().take(MAX_PDF_PAGES) {
        if let Ok(text) = doc.extract_text(&[page]) {
            let text = text.trim();
            if !text.is_empty() {
                out.push_str(text);
                out.push('\n');
            }
        }
    }
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

/// At most `max` characters, cut on a char boundary.
fn clip(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((cut, _)) => format!("{}\n[cut at {max} characters]", &text[..cut]),
        None => text.to_string(),
    }
}

/// The `<id@host>` tokens of a Message-ID / In-Reply-To / References
/// value, in order, brackets kept (the form we send and store).
pub fn message_ids(value: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = value;
    while let Some(open) = rest.find('<') {
        let Some(close) = rest[open..].find('>') else {
            break;
        };
        let id = &rest[open..open + close + 1];
        if id.len() > 2 && !id.contains(char::is_whitespace) {
            out.push(id.to_string());
        }
        rest = &rest[open + close + 1..];
    }
    out
}

/// Sender addr-spec from a `From:` value (`Name <addr>` or bare `addr`),
/// lowercased. Empty when unparseable.
///
/// Quoted display names and `(comments)` are dropped first, so neither
/// `"x <ops@sos.wa.gov>" <a@evil.test>` nor `<a@evil.test> (<ops@sos.wa.gov>)`
/// reads as the official address; of what remains, the last `<…>` is
/// the mailbox (an angle-addr ends a name-addr). A From naming several
/// mailboxes has no single sender and yields empty.
pub fn sender_addr(from: &str) -> String {
    let bare = strip_quotes_and_comments(from);
    let addr = match bare.rfind('<') {
        Some(open) => match bare[open..].find('>') {
            Some(close) => &bare[open + 1..open + close],
            None => return String::new(),
        },
        None => bare.trim(),
    };
    let outside_angle = match bare.rfind('<') {
        Some(open) => &bare[..open],
        None => bare.as_str(),
    };
    if outside_angle.contains([',', '<', '>']) || addr.contains([',', '<', '>']) {
        return String::new();
    }
    addr.trim().to_ascii_lowercase()
}

/// A header value with `"quoted strings"` and nested `(comments)`
/// removed (backslash escapes honored inside both). An unterminated
/// quote or comment drops the rest of the value.
fn strip_quotes_and_comments(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut in_quote = false;
    let mut depth = 0u32;
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        match ch {
            '\\' if in_quote || depth > 0 => {
                chars.next();
            }
            '"' if depth == 0 => in_quote = !in_quote,
            '(' if !in_quote => depth += 1,
            ')' if !in_quote && depth > 0 => depth -= 1,
            _ if !in_quote && depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out
}

/// Lowercased domain of an addr-spec, if any.
pub fn sender_domain(addr: &str) -> Option<String> {
    addr.rsplit_once('@')
        .map(|(_, domain)| domain.trim().to_ascii_lowercase())
        .filter(|d| !d.is_empty() && !d.contains([' ', '<', '>']))
}

fn split_head_body(text: &str) -> (&str, &str) {
    if let Some(idx) = text.find("\r\n\r\n") {
        return (&text[..idx], &text[idx + 4..]);
    }
    if let Some(idx) = text.find("\n\n") {
        return (&text[..idx], &text[idx + 2..]);
    }
    (text, "")
}

/// Unfolded `(lowercased-name, value)` headers, in transmission order
/// (also the field blocks of a delivery-status part).
pub(crate) fn header_fields(head: &str) -> Vec<(String, String)> {
    let mut headers = Vec::new();
    let mut current: Option<(String, String)> = None;
    for line in head.lines() {
        if line.starts_with([' ', '\t']) {
            if let Some((_, value)) = current.as_mut() {
                value.push(' ');
                value.push_str(line.trim());
            }
            continue;
        }
        if let Some(pair) = current.take() {
            headers.push(pair);
        }
        if let Some((name, value)) = line.split_once(':') {
            current = Some((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }
    if let Some(pair) = current.take() {
        headers.push(pair);
    }
    headers
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    const SIMPLE: &str = "From: ops@example.gov\r\nTo: box@example.com\r\nDate: Mon, 26 Oct 2026 09:00:00 -0700\r\nSubject: closure\r\nAuthentication-Results: mx.google.com; dkim=pass header.d=example.gov\r\n\r\nClosed Monday.";

    #[test]
    fn simple_headers_and_body() {
        let parsed = parse(SIMPLE.as_bytes());
        assert_eq!(parsed.from, "ops@example.gov");
        assert_eq!(parsed.to, "box@example.com");
        assert_eq!(parsed.subject, "closure");
        assert_eq!(parsed.body_text, "Closed Monday.");
        assert_eq!(parsed.auth_results.len(), 1);
        assert_eq!(sender_addr(&parsed.from), "ops@example.gov");
        assert_eq!(
            sender_domain("ops@example.gov").as_deref(),
            Some("example.gov")
        );
    }

    #[test]
    fn threading_headers_yield_bracketed_ids() {
        let raw = "From: ops@example.gov\r\nMessage-ID: <r-1@example.gov>\r\nIn-Reply-To: <outreach-4-ab@andrii.ai>\r\nReferences: <first@x.test>\r\n <outreach-4-ab@andrii.ai>\r\n\r\nYes.";
        let parsed = parse(raw.as_bytes());
        assert_eq!(message_ids(&parsed.message_id), ["<r-1@example.gov>"]);
        assert_eq!(
            message_ids(&parsed.in_reply_to),
            ["<outreach-4-ab@andrii.ai>"]
        );
        assert_eq!(
            message_ids(&parsed.references),
            ["<first@x.test>", "<outreach-4-ab@andrii.ai>"]
        );
        assert!(message_ids("no ids <> here").is_empty());
        assert!(message_ids("<a b@c>").is_empty());
    }

    #[test]
    fn folded_subject_and_named_sender() {
        let raw = "From: Ops Team <ops@example.gov>\r\nSubject: Office closure\r\n tomorrow announced\r\n\r\nBody.";
        let parsed = parse(raw.as_bytes());
        assert_eq!(parsed.subject, "Office closure tomorrow announced");
        assert_eq!(sender_addr(&parsed.from), "ops@example.gov");
    }

    #[test]
    fn multipart_prefers_text_plain() {
        let raw = concat!(
            "From: a@b.test\r\n",
            "Content-Type: multipart/alternative; boundary=\"b1\"\r\n",
            "\r\n",
            "--b1\r\nContent-Type: text/plain\r\n\r\nPlain words.\r\n",
            "--b1\r\nContent-Type: text/html\r\n\r\n<p>Html words.</p>\r\n",
            "--b1--\r\n",
        );
        let parsed = parse(raw.as_bytes());
        assert_eq!(parsed.body_text, "Plain words.");
    }

    #[test]
    fn base64_and_quoted_printable_bodies() {
        // "Hello there" / "Caf=C3=A9" (utf-8 bytes, lossy-kept).
        let b64 = "From: a@b.test\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: base64\r\n\r\nSGVsbG8gdGhlcmU=\r\n";
        assert_eq!(parse(b64.as_bytes()).body_text, "Hello there");
        let qp = "From: a@b.test\r\nContent-Type: text/plain\r\nContent-Transfer-Encoding: quoted-printable\r\n\r\nCaf=C3=A9 =\r\nand more";
        assert_eq!(parse(qp.as_bytes()).body_text, "Café and more");
    }

    #[test]
    fn sender_addr_ignores_addresses_hidden_in_names_and_comments() {
        // The display name quotes an official address; the mailbox is
        // the attacker's.
        assert_eq!(
            sender_addr("\"x <ops@sos.wa.gov>\" <a@evil.test>"),
            "a@evil.test"
        );
        // Unquoted display name carrying angle brackets: the last
        // angle-addr is the mailbox.
        assert_eq!(sender_addr("ops@sos.wa.gov <a@evil.test>"), "a@evil.test");
        // A comment after the mailbox may not supply one.
        assert_eq!(
            sender_addr("<a@evil.test> (<ops@sos.wa.gov>)"),
            "a@evil.test"
        );
        assert_eq!(
            sender_addr("\"Ops \\\"Team\\\" (WA)\" <Ops@Example.GOV>"),
            "ops@example.gov"
        );
        assert_eq!(sender_addr("ops@example.gov (Ops desk)"), "ops@example.gov");
        // Several mailboxes, or broken brackets: no single sender.
        assert_eq!(sender_addr("ops@sos.wa.gov, a@evil.test"), "");
        assert_eq!(sender_addr("<ops@sos.wa.gov>, <a@evil.test>"), "");
        assert_eq!(sender_addr("Ops <ops@example.gov"), "");
        assert_eq!(sender_addr("\"unterminated <ops@example.gov>"), "");
    }

    #[test]
    fn sender_domain_rejects_garbage() {
        assert_eq!(sender_domain("no-at-sign"), None);
        assert_eq!(sender_domain("a@"), None);
        assert_eq!(sender_domain("a@b c"), None);
        assert_eq!(
            sender_domain("A@Example.GOV").as_deref(),
            Some("example.gov")
        );
    }

    #[test]
    fn garbage_input_yields_empty_fields() {
        let parsed = parse(b"\xff\xfe binary \x00 blob");
        assert!(parsed.from.is_empty());
        assert!(parsed.auth_results.is_empty());
    }

    /// A one-page PDF whose text layer says `text` (Helvetica).
    pub(crate) fn pdf_with_text(text: &str) -> Vec<u8> {
        use lopdf::content::{Content, Operation};
        use lopdf::{Document, Object, Stream, dictionary};
        let mut doc = Document::with_version("1.7");
        let pages_id = doc.new_object_id();
        let font = doc.add_object(
            dictionary! {"Type" => "Font", "Subtype" => "Type1", "BaseFont" => "Helvetica"},
        );
        let content = Content {
            operations: vec![
                Operation::new("BT", vec![]),
                Operation::new("Tf", vec!["F1".into(), 12.into()]),
                Operation::new("Td", vec![72.into(), 720.into()]),
                Operation::new("Tj", vec![Object::string_literal(text)]),
                Operation::new("ET", vec![]),
            ],
        };
        let content_id = doc.add_object(Stream::new(dictionary! {}, content.encode().unwrap()));
        let page_id = doc.add_object(dictionary! {
            "Type" => "Page", "Parent" => Object::Reference(pages_id),
            "MediaBox" => vec![0.into(), 0.into(), 612.into(), 792.into()],
            "Contents" => Object::Reference(content_id),
            "Resources" => dictionary! {"Font" => dictionary! {"F1" => Object::Reference(font)}},
        });
        doc.objects.insert(
            pages_id,
            Object::Dictionary(dictionary! {"Type" => "Pages", "Kids" => vec![Object::Reference(page_id)], "Count" => 1}),
        );
        let catalog = doc
            .add_object(dictionary! {"Type" => "Catalog", "Pages" => Object::Reference(pages_id)});
        doc.trailer.set("Root", Object::Reference(catalog));
        let mut out = Vec::new();
        doc.save_to(&mut out).unwrap();
        out
    }

    fn b64(bytes: &[u8]) -> String {
        use base64::Engine as _;
        let encoded = base64::engine::general_purpose::STANDARD.encode(bytes);
        encoded
            .as_bytes()
            .chunks(76)
            .map(|c| std::str::from_utf8(c).unwrap())
            .collect::<Vec<_>>()
            .join("\r\n")
    }

    /// An office notice the way mail clients send one: mixed →
    /// (alternative: plain + html) + a PDF attachment, encoded subject.
    pub(crate) fn notice_with_pdf() -> String {
        format!(
            concat!(
                "From: ops@example.gov\r\n",
                "Subject: =?UTF-8?B?RmVlIGNoYW5nZSDigJQgMjAyNw==?=\r\n",
                "Content-Type: multipart/mixed; boundary=\"outer\"\r\n",
                "\r\n",
                "--outer\r\n",
                "Content-Type: multipart/alternative; boundary=\"inner\"\r\n",
                "\r\n",
                "--inner\r\nContent-Type: text/plain; charset=utf-8\r\n\r\nSee the attached notice.\r\n",
                "--inner\r\nContent-Type: text/html; charset=utf-8\r\n\r\n<p>See the <b>attached</b> notice.</p>\r\n",
                "--inner--\r\n",
                "--outer\r\n",
                "Content-Type: application/pdf; name=\"notice.pdf\"\r\n",
                "Content-Disposition: attachment; filename=\"notice.pdf\"\r\n",
                "Content-Transfer-Encoding: base64\r\n",
                "\r\n{}\r\n",
                "--outer--\r\n",
            ),
            b64(&pdf_with_text("Apostille fee is 30 dollars from 1 March"))
        )
    }

    #[test]
    fn nested_multipart_reads_the_body_and_the_pdf_text_layer() {
        let parsed = parse(notice_with_pdf().as_bytes());
        assert_eq!(parsed.subject, "Fee change \u{2014} 2027");
        assert!(
            parsed.body_text.starts_with("See the attached notice."),
            "{}",
            parsed.body_text
        );
        assert!(!parsed.body_text.contains("<b>"), "{}", parsed.body_text);
        assert!(
            parsed
                .body_text
                .contains("[Attachment: notice.pdf]\nApostille fee is 30 dollars from 1 March"),
            "{}",
            parsed.body_text
        );
        assert_eq!(parsed.attachments.len(), 1);
        assert_eq!(parsed.attachments[0].content_type, "application/pdf");
        assert!(parsed.attachments[0].chars > 20);
        // The crash-guard path: same message, PDFs left unread.
        let guarded = parse_with(notice_with_pdf().as_bytes(), false);
        assert!(
            !guarded.body_text.contains("Apostille fee"),
            "{}",
            guarded.body_text
        );
        assert_eq!(guarded.attachments[0].note, "skipped after a failed read");
    }

    #[test]
    fn charsets_and_encoded_words_decode() {
        // Latin-1 quoted-printable body, Q-encoded Latin-1 subject.
        let raw = concat!(
            "From: ops@example.fr\r\n",
            "Subject: =?ISO-8859-1?Q?Fermeture_du_bureau_=E0_Paris?=\r\n",
            "Content-Type: text/plain; charset=iso-8859-1\r\n",
            "Content-Transfer-Encoding: quoted-printable\r\n",
            "\r\n",
            "Le bureau est ferm=E9 lundi.\r\n",
        );
        let parsed = parse(raw.as_bytes());
        assert_eq!(parsed.subject, "Fermeture du bureau \u{e0} Paris");
        assert_eq!(parsed.body_text, "Le bureau est ferm\u{e9} lundi.");
        // Windows-1252 smart quotes and a Shift_JIS part.
        let raw = b"From: a@b.test\r\nContent-Type: text/plain; charset=windows-1252\r\n\r\n\x93Closed\x94 \x96 Monday";
        assert_eq!(
            parse(raw).body_text,
            "\u{201c}Closed\u{201d} \u{2013} Monday"
        );
        let raw = b"From: a@b.test\r\nContent-Type: text/plain; charset=shift_jis\r\n\r\n\x93\x8c\x8b\x9e";
        assert_eq!(parse(raw).body_text, "\u{6771}\u{4eac}");
    }

    #[test]
    fn forwarded_messages_and_text_attachments_are_read() {
        let raw = concat!(
            "From: staff@example.com\r\n",
            "Subject: Fwd: closure\r\n",
            "Content-Type: multipart/mixed; boundary=\"m\"\r\n",
            "\r\n",
            "--m\r\nContent-Type: text/plain\r\n\r\nForwarding the office's notice.\r\n",
            "--m\r\nContent-Type: message/rfc822\r\n\r\n",
            "From: ops@example.gov\r\nSubject: Office closed Friday\r\n",
            "Content-Type: multipart/alternative; boundary=\"a\"\r\n\r\n",
            "--a\r\nContent-Type: text/html\r\n\r\n<div>Closed <i>Friday</i> for the holiday.</div>\r\n--a--\r\n",
            "--m\r\nContent-Type: text/plain; name=\"hours.txt\"\r\nContent-Disposition: attachment; filename=\"hours.txt\"\r\n\r\nOpen 8-4 Mon-Thu.\r\n",
            "--m\r\nContent-Type: image/png; name=\"logo.png\"\r\nContent-Transfer-Encoding: base64\r\n\r\niVBORw0KGgo=\r\n",
            "--m--\r\n",
        );
        let parsed = parse(raw.as_bytes());
        let body = &parsed.body_text;
        assert!(
            body.starts_with("Forwarding the office's notice."),
            "{body}"
        );
        assert!(body.contains("Subject: Office closed Friday"), "{body}");
        assert!(body.contains("Closed Friday for the holiday."), "{body}");
        assert!(
            body.contains("[Attachment: hours.txt]\nOpen 8-4 Mon-Thu."),
            "{body}"
        );
        assert!(
            !body.contains("logo"),
            "images are layout, not content: {body}"
        );
    }

    #[test]
    fn a_scanned_pdf_is_noted_and_oversized_bodies_are_cut() {
        assert_eq!(clip("abcdef", 3), "abc\n[cut at 3 characters]");
        assert_eq!(clip("abc", 3), "abc");
        let raw = format!(
            "From: a@b.test\r\nContent-Type: multipart/mixed; boundary=\"m\"\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nSee scan.\r\n--m\r\nContent-Type: application/pdf; name=\"scan.pdf\"\r\nContent-Transfer-Encoding: base64\r\n\r\n{}\r\n--m--\r\n",
            b64(b"%PDF-1.4 not really a pdf")
        );
        let parsed = parse(raw.as_bytes());
        assert_eq!(parsed.body_text, "See scan.");
        assert_eq!(parsed.attachments[0].note, "no text layer");
    }
}
