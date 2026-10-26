//! Closed vocabulary of handbook fact topics.
//!
//! Every `fact.topic` value MUST be one of these [`Topic`]s. Meanings,
//! owning streams, and example questions live in `docs/specs/TOPICS.md`.
//! The enum is closed on purpose: [`Topic::parse`] rejects unknown input
//! with an error instead of guessing, so typos and attacker-controlled
//! strings can never invent a topic.

use std::fmt;
use std::str::FromStr;

/// A handbook fact topic (closed vocabulary).
///
/// The wire form is the snake_case string (e.g. `"po_box"`); see
/// [`Topic::as_str`]. `po_box` is deliberately its own topic, not part
/// of `mailing` (see `docs/specs/TOPICS.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    Hours,
    Fees,
    Forms,
    Address,
    Parking,
    RoutePhotos,
    Rejections,
    Contacts,
    PoBox,
    Walkin,
    Mailing,
    Appointment,
    Payment,
    ProcessingTime,
    Jurisdiction,
    LegalizationChain,
    Website,
    PhoneScript,
    HolidayHours,
    IdRequirements,
    Copies,
    Translation,
    Notarization,
    Authentication,
    Dropbox,
}

impl Topic {
    /// Every topic, for iteration and exhaustive tests.
    pub const ALL: [Topic; 25] = [
        Topic::Hours,
        Topic::Fees,
        Topic::Forms,
        Topic::Address,
        Topic::Parking,
        Topic::RoutePhotos,
        Topic::Rejections,
        Topic::Contacts,
        Topic::PoBox,
        Topic::Walkin,
        Topic::Mailing,
        Topic::Appointment,
        Topic::Payment,
        Topic::ProcessingTime,
        Topic::Jurisdiction,
        Topic::LegalizationChain,
        Topic::Website,
        Topic::PhoneScript,
        Topic::HolidayHours,
        Topic::IdRequirements,
        Topic::Copies,
        Topic::Translation,
        Topic::Notarization,
        Topic::Authentication,
        Topic::Dropbox,
    ];

    /// The snake_case wire form stored in `fact.topic`.
    pub fn as_str(self) -> &'static str {
        match self {
            Topic::Hours => "hours",
            Topic::Fees => "fees",
            Topic::Forms => "forms",
            Topic::Address => "address",
            Topic::Parking => "parking",
            Topic::RoutePhotos => "route_photos",
            Topic::Rejections => "rejections",
            Topic::Contacts => "contacts",
            Topic::PoBox => "po_box",
            Topic::Walkin => "walkin",
            Topic::Mailing => "mailing",
            Topic::Appointment => "appointment",
            Topic::Payment => "payment",
            Topic::ProcessingTime => "processing_time",
            Topic::Jurisdiction => "jurisdiction",
            Topic::LegalizationChain => "legalization_chain",
            Topic::Website => "website",
            Topic::PhoneScript => "phone_script",
            Topic::HolidayHours => "holiday_hours",
            Topic::IdRequirements => "id_requirements",
            Topic::Copies => "copies",
            Topic::Translation => "translation",
            Topic::Notarization => "notarization",
            Topic::Authentication => "authentication",
            Topic::Dropbox => "dropbox",
        }
    }

    /// Parse the snake_case wire form. Unknown input (typos, wrong
    /// case, surrounding whitespace, anything not in [`Topic::ALL`])
    /// is an [`UnknownTopic`] error, never a guess.
    pub fn parse(input: &str) -> Result<Self, UnknownTopic> {
        match input {
            "hours" => Ok(Topic::Hours),
            "fees" => Ok(Topic::Fees),
            "forms" => Ok(Topic::Forms),
            "address" => Ok(Topic::Address),
            "parking" => Ok(Topic::Parking),
            "route_photos" => Ok(Topic::RoutePhotos),
            "rejections" => Ok(Topic::Rejections),
            "contacts" => Ok(Topic::Contacts),
            "po_box" => Ok(Topic::PoBox),
            "walkin" => Ok(Topic::Walkin),
            "mailing" => Ok(Topic::Mailing),
            "appointment" => Ok(Topic::Appointment),
            "payment" => Ok(Topic::Payment),
            "processing_time" => Ok(Topic::ProcessingTime),
            "jurisdiction" => Ok(Topic::Jurisdiction),
            "legalization_chain" => Ok(Topic::LegalizationChain),
            "website" => Ok(Topic::Website),
            "phone_script" => Ok(Topic::PhoneScript),
            "holiday_hours" => Ok(Topic::HolidayHours),
            "id_requirements" => Ok(Topic::IdRequirements),
            "copies" => Ok(Topic::Copies),
            "translation" => Ok(Topic::Translation),
            "notarization" => Ok(Topic::Notarization),
            "authentication" => Ok(Topic::Authentication),
            "dropbox" => Ok(Topic::Dropbox),
            _ => Err(UnknownTopic(input.to_string())),
        }
    }
}

impl fmt::Display for Topic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl FromStr for Topic {
    type Err = UnknownTopic;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::parse(s)
    }
}

/// [`Topic::parse`] failure: the input names no known topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnknownTopic(String);

impl fmt::Display for UnknownTopic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown handbook topic: {:?}", self.0)
    }
}

impl std::error::Error for UnknownTopic {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_topics_roundtrip() {
        assert_eq!(Topic::ALL.len(), 25);
        for topic in Topic::ALL {
            assert_eq!(topic.to_string(), topic.as_str());
            assert_eq!(Topic::parse(topic.as_str()).unwrap(), topic);
            assert_eq!(topic.as_str().parse::<Topic>().unwrap(), topic);
        }
        assert_eq!(Topic::parse("po_box").unwrap(), Topic::PoBox);
        assert_eq!(Topic::Dropbox.to_string(), "dropbox");
    }

    #[test]
    fn parse_rejects_unknown_topics() {
        for bad in [
            "",
            "PO_BOX",
            "po-box",
            " hours",
            "mailing ",
            "Mailing",
            "bitcoin",
            "dropbox/../etc",
        ] {
            Topic::parse(bad).unwrap_err();
            bad.parse::<Topic>().unwrap_err();
        }
    }
}
