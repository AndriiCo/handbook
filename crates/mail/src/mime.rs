//! Minimal RFC 2822 parsing for the mail sidecar: headers of interest
//! + a text body.
//!
//! Deliberately narrow (slice scope, mirroring the naive-HTML-extraction
//! precedent): unfolded headers, one multipart level walked for the
//! first `text/plain` part (`text/html` tag-stripped as fallback),
//! `Content-Transfer-Encoding` base64 / quoted-printable /
//! 7bit-8bit-binary passthrough, `from_utf8_lossy` (no charset
//! transcoding). The raw bytes are always preserved as the capture
//! blob, so a fuller parser (`mail-parser` is the named upgrade) can
//! re-derive sidecars later without re-fetching.

/// What the poller needs from one message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedMail {
    pub from: String,
    pub to: String,
    pub date: String,
    pub subject: String,
    /// All `Authentication-Results` values, topmost first.
    pub auth_results: Vec<String>,
    /// Sidecar body: decoded `text/plain`, tag-stripped `text/html`
    /// fallback, else empty.
    pub body_text: String,
}

/// Parse headers + sidecar body from RFC 2822 bytes. Never fails:
/// unparseable input yields empty fields (the blob stays authoritative).
pub fn parse(raw: &[u8]) -> ParsedMail {
    let text = String::from_utf8_lossy(raw);
    let (head, body) = split_head_body(&text);
    let headers = parse_headers(head);
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
    let content_type: String = get("content-type");
    let body_text = extract_body(&content_type, &headers, body);
    ParsedMail {
        from: get("from"),
        to: get("to"),
        date: get("date"),
        subject: get("subject"),
        auth_results: all("authentication-results"),
        body_text,
    }
}

/// Sender addr-spec from a `From:` value (`Name <addr>` or bare `addr`),
/// lowercased. Empty when unparseable.
pub fn sender_addr(from: &str) -> String {
    let addr = match (from.find('<'), from.find('>')) {
        (Some(open), Some(close)) if open < close => &from[open + 1..close],
        _ => from.trim(),
    };
    addr.trim().to_ascii_lowercase()
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

/// Unfolded `(lowercased-name, value)` headers, in transmission order.
fn parse_headers(head: &str) -> Vec<(String, String)> {
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

fn extract_body(content_type: &str, headers: &[(String, String)], body: &str) -> String {
    let mime = content_type
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    if mime.starts_with("multipart/") {
        if let Some(boundary) = boundary_param(content_type) {
            return extract_multipart(&boundary, body);
        }
        return String::new();
    }
    let encoding = headers
        .iter()
        .find(|(n, _)| n == "content-transfer-encoding")
        .map(|(_, v)| v.to_ascii_lowercase())
        .unwrap_or_default();
    let decoded = decode_cte(&encoding, body);
    if mime == "text/html" {
        return ingest::capture::extract_text(&decoded);
    }
    String::from_utf8_lossy(&decoded).trim().to_string()
}

fn boundary_param(content_type: &str) -> Option<String> {
    for part in content_type.split(';').skip(1) {
        let part = part.trim();
        if let Some(value) = part
            .strip_prefix("boundary=")
            .or_else(|| part.strip_prefix("BOUNDARY="))
        {
            return Some(value.trim_matches('"').to_string());
        }
    }
    None
}

/// First `text/plain` part wins; else the first `text/html` part
/// tag-stripped; else empty. One level only (nested multiparts are
/// rare in office mail; the blob preserves everything anyway).
fn extract_multipart(boundary: &str, body: &str) -> String {
    let delimiter = format!("--{boundary}");
    let mut html_fallback: Option<String> = None;
    for chunk in body.split(&delimiter).skip(1) {
        let chunk = chunk.strip_suffix("--").unwrap_or(chunk);
        let (head, part_body) = split_head_body(chunk);
        let headers = parse_headers(head);
        let get = |name: &str| {
            headers
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
                .unwrap_or("")
        };
        let mime = get("content-type")
            .split(';')
            .next()
            .unwrap_or("")
            .trim()
            .to_ascii_lowercase();
        if mime.is_empty() || mime.starts_with("multipart/") {
            continue;
        }
        let decoded = decode_cte(
            &get("content-transfer-encoding").to_ascii_lowercase(),
            part_body,
        );
        if mime == "text/plain" {
            return String::from_utf8_lossy(&decoded).trim().to_string();
        }
        if mime == "text/html" && html_fallback.is_none() {
            html_fallback = Some(ingest::capture::extract_text(&decoded));
        }
    }
    html_fallback.unwrap_or_default()
}

/// Decode one `Content-Transfer-Encoding`. Unknown encodings pass
/// through (lossy bytes beat a failed sidecar).
fn decode_cte(encoding: &str, body: &str) -> Vec<u8> {
    match encoding.trim().to_ascii_lowercase().as_str() {
        "base64" => {
            let compact: String = body.chars().filter(|c| !c.is_whitespace()).collect();
            use base64::Engine as _;
            base64::engine::general_purpose::STANDARD
                .decode(&compact)
                .unwrap_or_else(|_| body.as_bytes().to_vec())
        }
        "quoted-printable" => decode_quoted_printable(body),
        _ => body.as_bytes().to_vec(),
    }
}

/// Minimal quoted-printable: `=XX` hex bytes, soft line breaks.
/// Malformed sequences pass through literally.
fn decode_quoted_printable(body: &str) -> Vec<u8> {
    let bytes = body.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'=' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        let rest = &bytes[i + 1..];
        // Soft break: `=\r\n` or `=\n` vanishes.
        if rest.starts_with(b"\r\n") {
            i += 3;
            continue;
        }
        if rest.starts_with(b"\n") {
            i += 2;
            continue;
        }
        if rest.len() >= 2 && rest[0].is_ascii_hexdigit() && rest[1].is_ascii_hexdigit() {
            let hex = |b: u8| (b as char).to_digit(16).unwrap_or(0) as u8;
            out.push(hex(rest[0]) << 4 | hex(rest[1]));
            i += 3;
        } else {
            out.push(b'=');
            i += 1;
        }
    }
    out
}

#[cfg(test)]
mod tests {
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
}
