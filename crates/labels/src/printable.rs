//! Option A renderer: customer-printable address labels.
//!
//! The output is a self-contained HTML document with `@page` print CSS
//! at the chosen layout size (the HTML-print-first roadmap, P2-R6).
//! The customer prints it from the browser and affixes postage
//! separately — every rendering states it is NOT postage, so a printed
//! Option A label can never be mistaken for a prepaid one.
//!
//! Rendering is deterministic: the same inputs produce byte-identical
//! output (no timestamps embedded).

use crate::carrier::Carrier;
use crate::format::PrintFormat;
use crate::payload::{Direction, PostalAddress, ShipmentPayload};

/// Render a printable label for one direction of a shipment.
/// `service_name` is the UI-safe name (e.g. `USPS Priority`).
pub fn render_printable(
    payload: &ShipmentPayload,
    direction: Direction,
    carrier: Carrier,
    format: PrintFormat,
    service_name: &str,
) -> String {
    let (from, to) = match direction {
        Direction::Outbound => (&payload.from, &payload.to),
        Direction::Return => (&payload.to, &payload.from),
    };
    let (page_size, content_width) = format.page_css();
    let title = match direction {
        Direction::Outbound => "Outbound label",
        Direction::Return => "Return label",
    };
    let body = match format {
        PrintFormat::WindowEnvelope => window_body(from, to),
        _ => label_body(from, to),
    };
    format!(
        "<!DOCTYPE html>\n\
         <html lang=\"en\">\n\
         <head>\n\
         <meta charset=\"utf-8\">\n\
         <title>{title} — {pairing}</title>\n\
         <style>\n\
         @page {{ size: {page_size}; margin: 0.2in; }}\n\
         body {{ font-family: sans-serif; color: #111; max-width: {content_width}; margin: 0; }}\n\
         .from {{ font-size: 11pt; margin-bottom: 1.2em; white-space: pre-line; }}\n\
         .to {{ font-size: 16pt; font-weight: bold; border: 2px solid #111; \
         padding: 0.6em; white-space: pre-line; }}\n\
         .meta {{ font-size: 9pt; margin-top: 1em; color: #333; }}\n\
         .not-postage {{ font-size: 10pt; font-weight: bold; margin-top: 1em; \
         border-top: 1px solid #111; padding-top: 0.5em; }}\n\
         </style>\n\
         </head>\n\
         <body>\n\
         <p class=\"meta\">{title} · {service} · {pairing}</p>\n\
         {body}\n\
         <p class=\"not-postage\">NOT POSTAGE — address label only. \
         Affix {carrier} postage before mailing.</p>\n\
         </body>\n\
         </html>\n",
        title = title,
        pairing = escape(&payload.pairing_id),
        page_size = page_size,
        content_width = content_width,
        service = escape(service_name),
        carrier = carrier,
        body = body,
    )
}

fn label_body(from: &PostalAddress, to: &PostalAddress) -> String {
    format!(
        "<p class=\"from\">From:\n{}</p>\n<p class=\"to\">To:\n{}</p>",
        escape(&address_text(from)),
        escape(&address_text(to)),
    )
}

/// Window-envelope layout: the return address sits top-left and the
/// recipient block is positioned for a standard #10 window.
fn window_body(from: &PostalAddress, to: &PostalAddress) -> String {
    format!(
        "<p class=\"from\">{}</p>\n\
         <div style=\"height: 1.2in;\"></div>\n\
         <p class=\"to\" style=\"margin-left: 1.5in;\">{}</p>",
        escape(&address_text(from)),
        escape(&address_text(to)),
    )
}

fn address_text(address: &PostalAddress) -> String {
    let mut lines = vec![
        address.name.clone(),
        address.street1.clone(),
        format!(
            "{}, {} {}",
            address.city, address.state_province, address.postal
        ),
    ];
    if let Some(street2) = address.street2.as_deref()
        && !street2.trim().is_empty()
    {
        lines.insert(2, street2.to_string());
    }
    lines.join("\n")
}

/// Minimal HTML escaping (text nodes only; no attribute interpolation
/// anywhere in the template).
fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::payload::tests::sample_payload;

    #[test]
    fn printable_shows_addresses_and_not_postage() {
        let html = render_printable(
            &sample_payload(),
            Direction::Outbound,
            Carrier::Usps,
            PrintFormat::FourBySix,
            "USPS Priority",
        );
        assert!(html.contains("Jane Doe"), "{html}");
        assert!(html.contains("WA Secretary of State"), "{html}");
        assert!(html.contains("NOT POSTAGE"), "{html}");
        assert!(html.contains("Affix USPS postage"), "{html}");
        assert!(html.contains("@page { size: 4in 6in;"), "{html}");
        assert!(html.contains("ship-synthetic-001"), "{html}");
    }

    #[test]
    fn return_direction_swaps_ends() {
        let html = render_printable(
            &sample_payload(),
            Direction::Return,
            Carrier::Usps,
            PrintFormat::Letter,
            "USPS Priority",
        );
        let from_pos = html.find("WA Secretary of State").unwrap();
        let to_pos = html.find("Jane Doe").unwrap();
        assert!(
            from_pos < to_pos,
            "return label reads consulate-first: {html}"
        );
    }

    #[test]
    fn window_layout_offsets_recipient() {
        let html = render_printable(
            &sample_payload(),
            Direction::Outbound,
            Carrier::Usps,
            PrintFormat::WindowEnvelope,
            "USPS Priority",
        );
        assert!(html.contains("margin-left: 1.5in;"), "{html}");
    }

    #[test]
    fn addresses_are_escaped() {
        let mut payload = sample_payload();
        payload.to.name = "<script>alert(1)</script>".to_string();
        let html = render_printable(
            &payload,
            Direction::Outbound,
            Carrier::Usps,
            PrintFormat::A4,
            "USPS Priority",
        );
        assert!(!html.contains("<script>"), "{html}");
        assert!(html.contains("&lt;script&gt;"), "{html}");
    }

    #[test]
    fn rendering_is_deterministic() {
        let payload = sample_payload();
        let a = render_printable(
            &payload,
            Direction::Outbound,
            Carrier::FedEx,
            PrintFormat::HalfLetter,
            "FedEx Ground",
        );
        let b = render_printable(
            &payload,
            Direction::Outbound,
            Carrier::FedEx,
            PrintFormat::HalfLetter,
            "FedEx Ground",
        );
        assert_eq!(a, b);
    }
}
