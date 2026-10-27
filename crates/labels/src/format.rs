//! Print layouts (Option A) and prepaid paper sizes (Option B).
//!
//! Option A is a customer-printable address label — the customer prints
//! it and affixes postage separately — so it offers every common layout
//! including a window-envelope one. Option B is a fully-paid USPS label
//! (digital stamp); its paper sizes are the subset the USPS Labels API
//! accepts for PDF output.

use serde::{Deserialize, Serialize};

/// Customer-printable layouts (Option A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PrintFormat {
    /// 4x6 in thermal/label stock (the default).
    FourBySix,
    /// Half of US Letter (8.5x5.5 in): two labels per sheet.
    HalfLetter,
    /// Full US Letter page (8.5x11 in).
    Letter,
    /// Full A4 page (210x297 mm).
    A4,
    /// #10 window envelope: return address top-left, recipient placed
    /// for the window.
    WindowEnvelope,
}

/// Prepaid USPS paper sizes (Option B).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum PaperSize {
    FourBySix,
    Letter,
    A4,
}

impl PrintFormat {
    /// Human name for the format picker. No prices, no dates.
    pub fn name(self) -> &'static str {
        match self {
            PrintFormat::FourBySix => "4×6 label",
            PrintFormat::HalfLetter => "Half-letter (two per sheet)",
            PrintFormat::Letter => "US Letter",
            PrintFormat::A4 => "A4",
            PrintFormat::WindowEnvelope => "Window envelope (#10)",
        }
    }

    /// CSS `@page` size + content width for the printable renderer.
    pub fn page_css(self) -> (&'static str, &'static str) {
        match self {
            PrintFormat::FourBySix => ("4in 6in", "3.6in"),
            PrintFormat::HalfLetter => ("8.5in 5.5in", "7.7in"),
            PrintFormat::Letter => ("letter", "7.5in"),
            PrintFormat::A4 => ("A4", "170mm"),
            PrintFormat::WindowEnvelope => ("letter", "7.5in"),
        }
    }

    /// Parse a stored/profile value. Unknown values are `None` so a
    /// bad profile row fails closed to the default, never to a panic.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "four_by_six" => Some(PrintFormat::FourBySix),
            "half_letter" => Some(PrintFormat::HalfLetter),
            "letter" => Some(PrintFormat::Letter),
            "a4" => Some(PrintFormat::A4),
            "window_envelope" => Some(PrintFormat::WindowEnvelope),
            _ => None,
        }
    }

    /// Stable storage spelling (matches the serde form).
    pub fn as_str(self) -> &'static str {
        match self {
            PrintFormat::FourBySix => "four_by_six",
            PrintFormat::HalfLetter => "half_letter",
            PrintFormat::Letter => "letter",
            PrintFormat::A4 => "a4",
            PrintFormat::WindowEnvelope => "window_envelope",
        }
    }
}

impl PaperSize {
    /// Human name for the paper picker.
    pub fn name(self) -> &'static str {
        match self {
            PaperSize::FourBySix => "4×6",
            PaperSize::Letter => "US Letter",
            PaperSize::A4 => "A4",
        }
    }

    /// Value sent in the USPS `labelSpecification` (verify the exact
    /// enum spelling against the v3 spec at integration time).
    pub fn usps_label_option(self) -> &'static str {
        match self {
            PaperSize::FourBySix => "4X6",
            PaperSize::Letter => "LETTER",
            PaperSize::A4 => "A4",
        }
    }

    /// Parse a stored/profile value; unknown values fail closed.
    pub fn parse(value: &str) -> Option<Self> {
        match value {
            "four_by_six" => Some(PaperSize::FourBySix),
            "letter" => Some(PaperSize::Letter),
            "a4" => Some(PaperSize::A4),
            _ => None,
        }
    }

    /// Stable storage spelling (matches the serde form).
    pub fn as_str(self) -> &'static str {
        match self {
            PaperSize::FourBySix => "four_by_six",
            PaperSize::Letter => "letter",
            PaperSize::A4 => "a4",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_round_trip_through_storage_spelling() {
        for format in [
            PrintFormat::FourBySix,
            PrintFormat::HalfLetter,
            PrintFormat::Letter,
            PrintFormat::A4,
            PrintFormat::WindowEnvelope,
        ] {
            assert_eq!(PrintFormat::parse(format.as_str()), Some(format));
            let json = serde_json::to_string(&format).unwrap();
            assert_eq!(json, format!("\"{}\"", format.as_str()));
        }
        for paper in [PaperSize::FourBySix, PaperSize::Letter, PaperSize::A4] {
            assert_eq!(PaperSize::parse(paper.as_str()), Some(paper));
            let json = serde_json::to_string(&paper).unwrap();
            assert_eq!(json, format!("\"{}\"", paper.as_str()));
        }
    }

    #[test]
    fn unknown_profile_values_fail_closed() {
        assert_eq!(PrintFormat::parse("scroll"), None);
        assert_eq!(PrintFormat::parse(""), None);
        assert_eq!(PaperSize::parse("half_letter"), None);
        assert_eq!(PaperSize::parse(""), None);
    }

    #[test]
    fn page_css_mentions_real_sizes() {
        assert_eq!(PrintFormat::FourBySix.page_css().0, "4in 6in");
        assert_eq!(PrintFormat::Letter.page_css().0, "letter");
        assert_eq!(PrintFormat::A4.page_css().0, "A4");
    }
}
