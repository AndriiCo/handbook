//! Delivery reports in the polled mailbox: bounces (RFC 3464,
//! `multipart/report; report-type=delivery-status`) and complaints
//! (RFC 5965 ARF, `report-type=feedback-report`).
//!
//! Outreach and digests go out through the same Gmail account the poll
//! reads, so its bounces land here. The poller feeds the recipients of
//! an authenticated report into `notify::suppress` and never stores
//! the report as a capture (it quotes a follower's address back).

use mail_parser::{Message, MimeHeaders, PartType};

/// Bounce or complaint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportKind {
    Bounce,
    Complaint,
}

impl ReportKind {
    pub fn as_str(self) -> &'static str {
        match self {
            ReportKind::Bounce => "bounce",
            ReportKind::Complaint => "complaint",
        }
    }
}

/// What one report says.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub kind: ReportKind,
    /// Recipients that failed permanently (`Action: failed`; a
    /// `delayed` notice is not a bounce) or that complained, lowercased.
    pub recipients: Vec<String>,
    /// `Message-ID`s of the original message, from the returned copy.
    pub original_ids: Vec<String>,
}

/// The report a parsed message carries, if it is one. `x_failed` is
/// the root `X-Failed-Recipients` value (Gmail stamps it on its own
/// bounces), used when the status part names nobody.
pub(crate) fn from_message(message: &Message<'_>, x_failed: &str) -> Option<Report> {
    let root = message.root_part().content_type()?;
    if !root.ctype().eq_ignore_ascii_case("multipart")
        || !root
            .subtype()
            .is_some_and(|s| s.eq_ignore_ascii_case("report"))
    {
        return None;
    }
    let kind = match root.attribute("report-type").map(str::to_ascii_lowercase) {
        Some(t) if t == "delivery-status" => ReportKind::Bounce,
        Some(t) if t == "feedback-report" => ReportKind::Complaint,
        _ => return None,
    };
    let mut recipients = Vec::new();
    let mut original_ids = Vec::new();
    let mut original_to = Vec::new();
    for part in &message.parts {
        let Some(ct) = part.content_type() else {
            continue;
        };
        let full = format!(
            "{}/{}",
            ct.ctype().to_ascii_lowercase(),
            ct.subtype().unwrap_or("").to_ascii_lowercase()
        );
        match full.as_str() {
            "message/delivery-status" | "message/global-delivery-status"
                if kind == ReportKind::Bounce =>
            {
                recipients.extend(failed_recipients(&String::from_utf8_lossy(part.contents())));
            }
            "message/feedback-report" if kind == ReportKind::Complaint => {
                let fields = crate::mime::header_fields(&String::from_utf8_lossy(part.contents()));
                recipients.extend(
                    fields
                        .iter()
                        .filter(|(n, _)| n == "original-rcpt-to")
                        .map(|(_, v)| addr_of(v)),
                );
            }
            "text/rfc822-headers" | "message/global-headers" => {
                let fields = crate::mime::header_fields(&String::from_utf8_lossy(part.contents()));
                for (name, value) in &fields {
                    match name.as_str() {
                        "message-id" => original_ids.extend(crate::mime::message_ids(value)),
                        "to" => original_to.push(addr_of(value)),
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        if let PartType::Message(original) = &part.body {
            if let Some(id) = original.message_id() {
                original_ids.push(format!("<{}>", id.trim_matches(['<', '>'])));
            }
            if let Some(to) = original
                .to()
                .and_then(|a| a.first())
                .and_then(|a| a.address())
            {
                original_to.push(to.to_ascii_lowercase());
            }
        }
    }
    if recipients.is_empty() {
        recipients = match kind {
            ReportKind::Bounce => x_failed.split(',').map(addr_of).collect(),
            // ARF without Original-Rcpt-To: the complainer is whoever
            // the returned copy was addressed to.
            ReportKind::Complaint => original_to,
        };
    }
    recipients.retain(|r| crate::mime::sender_domain(r).is_some() && !r.contains(' '));
    recipients.sort();
    recipients.dedup();
    original_ids.dedup();
    Some(Report {
        kind,
        recipients,
        original_ids,
    })
}

/// `Final-Recipient` (else `Original-Recipient`) of every per-recipient
/// block whose `Action` is `failed`.
fn failed_recipients(status: &str) -> Vec<String> {
    let normalized = status.replace("\r\n", "\n");
    let mut out = Vec::new();
    for block in normalized.split("\n\n") {
        let fields = crate::mime::header_fields(block);
        let get = |name: &str| {
            fields
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, v)| v.as_str())
        };
        if !get("action").is_some_and(|a| a.trim().eq_ignore_ascii_case("failed")) {
            continue;
        }
        if let Some(recipient) = get("final-recipient").or_else(|| get("original-recipient")) {
            out.push(addr_of(recipient));
        }
    }
    out
}

/// `rfc822; <a@b>` / `<a@b>` / `a@b` → `a@b`, lowercased.
fn addr_of(value: &str) -> String {
    let value = value.split_once(';').map(|(_, v)| v).unwrap_or(value);
    crate::mime::sender_addr(value)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn parse(raw: &str) -> Option<Report> {
        crate::mime::parse(raw.as_bytes()).report
    }

    /// Gmail's own bounce shape (fictional addresses).
    pub(crate) const GMAIL_BOUNCE: &str = concat!(
        "From: Mail Delivery Subsystem <mailer-daemon@googlemail.com>\r\n",
        "To: box@example.com\r\n",
        "Subject: Delivery Status Notification (Failure)\r\n",
        "X-Failed-Recipients: gone@follower.example\r\n",
        "Authentication-Results: mx.google.com; dkim=pass header.i=@googlemail.com header.s=20230601\r\n",
        "Content-Type: multipart/report; boundary=\"r1\"; report-type=delivery-status\r\n",
        "\r\n",
        "--r1\r\nContent-Type: text/plain; charset=UTF-8\r\n\r\n",
        "Address not found: gone@follower.example\r\n",
        "--r1\r\nContent-Type: message/delivery-status\r\n\r\n",
        "Reporting-MTA: dns; googlemail.com\r\n",
        "\r\n",
        "Final-Recipient: rfc822; Gone@Follower.example\r\n",
        "Action: failed\r\n",
        "Status: 5.1.1\r\n",
        "\r\n",
        "Final-Recipient: rfc822; slow@follower.example\r\n",
        "Action: delayed\r\n",
        "Status: 4.4.1\r\n",
        "\r\n",
        "--r1\r\nContent-Type: text/rfc822-headers\r\n\r\n",
        "From: notices@example.com\r\n",
        "To: gone@follower.example\r\n",
        "Message-ID: <digest-9-ab@andrii.ai>\r\n",
        "Subject: Apostille update\r\n",
        "--r1--\r\n",
    );

    #[test]
    fn bounce_names_only_permanent_failures() {
        let report = parse(GMAIL_BOUNCE).expect("report");
        assert_eq!(report.kind, ReportKind::Bounce);
        assert_eq!(report.recipients, ["gone@follower.example"]);
        assert_eq!(report.original_ids, ["<digest-9-ab@andrii.ai>"]);
    }

    #[test]
    fn bounce_without_status_fields_falls_back_to_x_failed_recipients() {
        let raw = GMAIL_BOUNCE.replace("Action: failed", "Action: delivered");
        assert_eq!(parse(&raw).unwrap().recipients, ["gone@follower.example"]);
    }

    #[test]
    fn complaint_reads_original_rcpt_to_or_the_returned_copy() {
        let arf = |extra: &str| {
            format!(
                "From: fbl@isp.example\r\nContent-Type: multipart/report; report-type=feedback-report; boundary=\"f\"\r\n\r\n\
                 --f\r\nContent-Type: text/plain\r\n\r\nAbuse report.\r\n\
                 --f\r\nContent-Type: message/feedback-report\r\n\r\nFeedback-Type: abuse\r\nVersion: 1\r\n{extra}\r\n\
                 --f\r\nContent-Type: message/rfc822\r\n\r\nFrom: notices@example.com\r\nTo: Annoyed@Follower.example\r\nMessage-ID: <digest-3-cd@andrii.ai>\r\nSubject: s\r\n\r\nbody\r\n\
                 --f--\r\n"
            )
        };
        let report = parse(&arf("Original-Rcpt-To: rcpt@follower.example\r\n")).unwrap();
        assert_eq!(report.kind, ReportKind::Complaint);
        assert_eq!(report.recipients, ["rcpt@follower.example"]);
        assert_eq!(report.original_ids, ["<digest-3-cd@andrii.ai>"]);
        let report = parse(&arf("")).unwrap();
        assert_eq!(report.recipients, ["annoyed@follower.example"]);
    }

    #[test]
    fn ordinary_mail_is_not_a_report() {
        assert_eq!(parse("From: a@b.test\r\nSubject: hi\r\n\r\nHello."), None);
        assert_eq!(
            parse(
                "From: a@b.test\r\nContent-Type: multipart/report; report-type=disposition-notification; boundary=x\r\n\r\n--x--\r\n"
            ),
            None
        );
    }
}
