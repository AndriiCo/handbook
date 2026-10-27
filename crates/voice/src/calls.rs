//! Local call states plus the forward-only transition rule.
//!
//! Telnyx webhooks are unordered and may duplicate, so a transition
//! applies only when it moves the call forward; anything else is a
//! no-op (`None`).

use std::fmt;

/// Call lifecycle state, stored as text on `voice_call.status`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CallStatus {
    Queued,
    Ringing,
    InProgress,
    Completed,
    Failed,
}

impl CallStatus {
    /// Stored/transmitted spelling (`queued`, `ringing`, …).
    pub fn as_str(self) -> &'static str {
        match self {
            CallStatus::Queued => "queued",
            CallStatus::Ringing => "ringing",
            CallStatus::InProgress => "in_progress",
            CallStatus::Completed => "completed",
            CallStatus::Failed => "failed",
        }
    }

    /// Parse a stored status; unknown spellings fail closed (`None`).
    pub fn parse(input: &str) -> Option<Self> {
        match input {
            "queued" => Some(CallStatus::Queued),
            "ringing" => Some(CallStatus::Ringing),
            "in_progress" => Some(CallStatus::InProgress),
            "completed" => Some(CallStatus::Completed),
            "failed" => Some(CallStatus::Failed),
            _ => None,
        }
    }

    /// Terminal states never move again (webhooks for terminal calls
    /// only attach artifacts — recordings, transcript segments).
    pub fn is_terminal(self) -> bool {
        matches!(self, CallStatus::Completed | CallStatus::Failed)
    }

    /// Forward-only transition for a webhook event. Returns the new
    /// status, or `None` when the event moves nothing (duplicate or
    /// out-of-order delivery).
    pub fn apply_event(self, event: &crate::EventType) -> Option<CallStatus> {
        if self.is_terminal() {
            return None;
        }
        let next = match event {
            crate::EventType::Initiated => CallStatus::Ringing,
            crate::EventType::Answered => CallStatus::InProgress,
            crate::EventType::Hangup => CallStatus::Completed,
            _ => return None,
        };
        let rank = |s: CallStatus| match s {
            CallStatus::Queued => 0,
            CallStatus::Ringing => 1,
            CallStatus::InProgress => 2,
            CallStatus::Completed | CallStatus::Failed => 3,
        };
        (rank(next) > rank(self)).then_some(next)
    }
}

impl fmt::Display for CallStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// E.164 check for dial targets: `+` plus 7–15 digits, nothing else.
/// This slice dials US offices only; the shape still rejects
/// garbage, letters, and separator tricks.
pub fn validate_e164(to: &str) -> Result<(), E164Error> {
    let digits = to.strip_prefix('+').ok_or(E164Error)?;
    if !(7..=15).contains(&digits.len()) || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(E164Error);
    }
    Ok(())
}

/// The dial target is not E.164 (`+` plus 7–15 digits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct E164Error;

impl fmt::Display for E164Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("number must be E.164 (+ plus 7-15 digits)")
    }
}

impl std::error::Error for E164Error {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::EventType;

    #[test]
    fn happy_path_moves_forward() {
        let mut status = CallStatus::Queued;
        for (event, want) in [
            (EventType::Initiated, CallStatus::Ringing),
            (EventType::Answered, CallStatus::InProgress),
            (EventType::Hangup, CallStatus::Completed),
        ] {
            status = status
                .apply_event(&event)
                .unwrap_or_else(|| panic!("{event:?} should move {status}"));
            assert_eq!(status, want);
        }
        assert!(status.is_terminal());
    }

    #[test]
    fn duplicates_and_reorder_are_noops() {
        let live = CallStatus::InProgress;
        assert_eq!(live.apply_event(&EventType::Answered), None);
        assert_eq!(live.apply_event(&EventType::Initiated), None);
        assert_eq!(live.apply_event(&EventType::Transcription), None);
        assert_eq!(live.apply_event(&EventType::RecordingSaved), None);
    }

    #[test]
    fn hangup_before_answer_completes() {
        assert_eq!(
            CallStatus::Queued.apply_event(&EventType::Hangup),
            Some(CallStatus::Completed)
        );
    }

    #[test]
    fn terminal_states_never_move() {
        for terminal in [CallStatus::Completed, CallStatus::Failed] {
            assert!(terminal.is_terminal());
            for event in [EventType::Initiated, EventType::Answered, EventType::Hangup] {
                assert_eq!(terminal.apply_event(&event), None);
            }
        }
    }

    #[test]
    fn status_round_trips() {
        for status in [
            CallStatus::Queued,
            CallStatus::Ringing,
            CallStatus::InProgress,
            CallStatus::Completed,
            CallStatus::Failed,
        ] {
            assert_eq!(CallStatus::parse(status.as_str()), Some(status));
        }
        assert_eq!(CallStatus::parse("bogus"), None);
    }

    #[test]
    fn e164_accepts_us_office_numbers() {
        assert!(validate_e164("+13605550100").is_ok());
        for bad in [
            "",
            "3605550100",
            "+1",
            "+1360555010000000",
            "+1-360-555-0100",
            "+1360555010a",
            "++13605550100",
        ] {
            assert!(validate_e164(bad).is_err(), "{bad}");
        }
    }
}
