//! The shared guest login: `demo@andrii.ai` signs in without a passkey
//! until the end of 2026-11-20, Pacific time. One fixed customer
//! account, so guests share its orders and it can never reach another
//! customer's data or the staff console.
//!
//! The deadline is a compiled constant checked at sign-in and on every
//! session use ([`crate::session_usable`]), so a session opened just
//! before it dies with it. Nothing has to run to turn this off.

/// The address that signs in as the guest.
pub const GUEST_EMAIL: &str = "demo@andrii.ai";

/// The guest account's `user_id` and `customer_ref`.
pub const GUEST_USER_ID: &str = "d3300000000040008000000000000001";

/// First second the guest login no longer works:
/// 2026-11-21 00:00:00 PST (08:00:00 UTC).
pub const GUEST_ENDS_AT: i64 = 1_795_248_000;

/// Last day the guest login works, as the login page shows it.
pub const GUEST_LAST_DAY: &str = "2026-11-20";

/// True while the guest login works.
pub fn guest_active(now: i64) -> bool {
    now < GUEST_ENDS_AT
}

/// True for the guest address, however it was typed.
pub fn is_guest_email(input: &str) -> bool {
    input.trim().eq_ignore_ascii_case(GUEST_EMAIL)
}

/// True for the guest account's `user_id`.
pub fn is_guest_user(user_id: &str) -> bool {
    user_id == GUEST_USER_ID
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deadline_is_the_end_of_november_20_pacific() {
        // 2026-11-21 08:00:00 UTC: whole days since the epoch, plus 8h.
        assert_eq!(GUEST_ENDS_AT, 20_778 * 86_400 + 8 * 3_600);
        assert!(guest_active(GUEST_ENDS_AT - 1));
        assert!(!guest_active(GUEST_ENDS_AT));
        assert!(!guest_active(GUEST_ENDS_AT + 86_400));
    }

    #[test]
    fn only_the_guest_address_matches() {
        assert!(is_guest_email("demo@andrii.ai"));
        assert!(is_guest_email("  DEMO@Andrii.AI "));
        assert!(!is_guest_email("demo@andrii.ai.evil.test"));
        assert!(!is_guest_email("demo@andrii.online"));
        assert!(!is_guest_email(""));
    }
}
