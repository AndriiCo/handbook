//! Login methods, account-linking rules, and provider signals.
//!
//! Pure policy from `docs/auth-playbook.md` §1–§6. HTTP handlers live
//! in the web crate; the rules here are what those handlers enforce,
//! so the anti-takeover core is testable without a server.

/// Customer login method, in playbook order (passkey first).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum LoginMethod {
    Passkey,
    Apple,
    Google,
    Meta,
    /// Email click-to-login; fallback only.
    MagicLink,
}

impl LoginMethod {
    /// Wire form stored in `identity_link.method`.
    pub fn as_str(self) -> &'static str {
        match self {
            LoginMethod::Passkey => "passkey",
            LoginMethod::Apple => "apple",
            LoginMethod::Google => "google",
            LoginMethod::Meta => "meta",
            LoginMethod::MagicLink => "magic_link",
        }
    }

    /// Parse the wire form; unknown input is `None`.
    pub fn parse(input: &str) -> Option<Self> {
        match input {
            "passkey" => Some(LoginMethod::Passkey),
            "apple" => Some(LoginMethod::Apple),
            "google" => Some(LoginMethod::Google),
            "meta" => Some(LoginMethod::Meta),
            "magic_link" => Some(LoginMethod::MagicLink),
            _ => None,
        }
    }
}

impl std::fmt::Display for LoginMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// One verified login identity (a row in `identity_link`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub method: LoginMethod,
    /// Provider subject: passkey credential id, Apple/Google `sub`,
    /// Meta app-scoped id, or the email address for magic links.
    pub provider_sub: String,
    /// Verified email, if the provider supplied one.
    pub verified_email: Option<String>,
    /// Apple private-relay address (distinct identity, never matched).
    pub is_relay: bool,
}

/// Email usable for verified-email matching (playbook §6 rules 2–3):
/// verified, non-empty, non-relay, and never Meta-supplied.
pub fn email_usable_for_matching(id: &Identity) -> bool {
    id.method != LoginMethod::Meta
        && !id.is_relay
        && id
            .verified_email
            .as_deref()
            .is_some_and(|email| !email.is_empty())
}

/// What to do with an unknown provider `sub`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NewIdentityDecision {
    /// No usable email match: mint a fresh account.
    CreateFreshAccount,
    /// A usable email match exists: do NOT merge — surface
    /// "an account with this email exists, sign in with X to link"
    /// (rule 5 kills pre-account takeover).
    PromptExplicitLink,
}

/// Route an unknown provider `sub`. `usable_email_match` must already
/// exclude relay/Meta/unverified emails via
/// [`email_usable_for_matching`].
pub fn decide_unknown_sub(usable_email_match: bool) -> NewIdentityDecision {
    if usable_email_match {
        NewIdentityDecision::PromptExplicitLink
    } else {
        NewIdentityDecision::CreateFreshAccount
    }
}

/// Explicit link flow (rule 4): active session on the old method +
/// auth on the new one + step-up. Only the step-up is this
/// function's input; the caller proves the two auths.
pub fn explicit_link_permitted(step_up_ok: bool) -> bool {
    step_up_ok
}

/// Unlinking (rule 6): a verified method must remain, plus re-auth.
pub fn unlink_permitted(remaining_methods: usize, reauthed: bool) -> bool {
    remaining_methods >= 1 && reauthed
}

/// Provider revocation / lifecycle signal (playbook §2–§4 tables).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProviderSignal {
    AppleEmailToggled,
    AppleConsentRevoked,
    AppleAccountDeleted,
    GoogleSessionsRevoked,
    GoogleCredentialChangeRequired,
    GoogleAccountDisabled,
    GoogleAccountEnabled,
    GoogleVerification,
    MetaDeauthorize,
    MetaDataDeletion,
}

/// Server action for one signal. Every signal is audited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalAction {
    pub disable_link: bool,
    pub kill_sessions: bool,
    pub force_step_up: bool,
    pub start_retention: bool,
    pub audit: &'static str,
}

/// Map each playbook signal to its server action.
pub fn signal_action(signal: ProviderSignal) -> SignalAction {
    match signal {
        ProviderSignal::AppleEmailToggled => SignalAction {
            disable_link: false,
            kill_sessions: false,
            force_step_up: false,
            start_retention: false,
            audit: "apple_email_toggled",
        },
        ProviderSignal::AppleConsentRevoked => SignalAction {
            disable_link: true,
            kill_sessions: true,
            force_step_up: false,
            start_retention: false,
            audit: "apple_consent_revoked",
        },
        ProviderSignal::AppleAccountDeleted => SignalAction {
            disable_link: true,
            kill_sessions: true,
            force_step_up: false,
            start_retention: true,
            audit: "apple_account_deleted",
        },
        ProviderSignal::GoogleSessionsRevoked => SignalAction {
            disable_link: false,
            kill_sessions: true,
            force_step_up: true,
            start_retention: false,
            audit: "google_sessions_revoked",
        },
        ProviderSignal::GoogleCredentialChangeRequired => SignalAction {
            disable_link: false,
            kill_sessions: false,
            force_step_up: true,
            start_retention: false,
            audit: "google_credential_change_required",
        },
        ProviderSignal::GoogleAccountDisabled => SignalAction {
            disable_link: true,
            kill_sessions: true,
            force_step_up: false,
            start_retention: false,
            audit: "google_account_disabled",
        },
        ProviderSignal::GoogleAccountEnabled => SignalAction {
            disable_link: false,
            kill_sessions: false,
            force_step_up: false,
            start_retention: false,
            audit: "google_account_enabled",
        },
        ProviderSignal::GoogleVerification => SignalAction {
            disable_link: false,
            kill_sessions: false,
            force_step_up: false,
            start_retention: false,
            audit: "google_risc_verification",
        },
        ProviderSignal::MetaDeauthorize => SignalAction {
            disable_link: true,
            kill_sessions: true,
            force_step_up: false,
            start_retention: false,
            audit: "meta_deauthorize",
        },
        ProviderSignal::MetaDataDeletion => SignalAction {
            disable_link: true,
            kill_sessions: true,
            force_step_up: false,
            start_retention: true,
            audit: "meta_data_deletion",
        },
    }
}

/// Magic-link token lifetime, seconds (15 min, single-use).
pub const MAGIC_LINK_TTL_SECS: i64 = 900;

/// Fresh 256-bit magic-link token, hex (bound to one email by storage).
pub fn mint_magic_token() -> String {
    use rand::RngCore as _;
    let mut bytes = [0u8; 32];
    rand::rng().fill_bytes(&mut bytes);
    let mut out = String::with_capacity(64);
    for b in bytes {
        out.push_str(&format!("{b:02x}"));
    }
    out
}

/// SHA-256 hex of a magic-link token for storage (tokens are
/// single-use bearer secrets; the DB keeps hashes only).
pub fn token_hash(token: &str) -> String {
    crate::hash_str(token)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn identity(method: LoginMethod, email: Option<&str>, relay: bool) -> Identity {
        Identity {
            method,
            provider_sub: "sub-1".to_string(),
            verified_email: email.map(str::to_string),
            is_relay: relay,
        }
    }

    #[test]
    fn methods_roundtrip() {
        for method in [
            LoginMethod::Passkey,
            LoginMethod::Apple,
            LoginMethod::Google,
            LoginMethod::Meta,
            LoginMethod::MagicLink,
        ] {
            assert_eq!(LoginMethod::parse(method.as_str()), Some(method));
        }
        assert_eq!(LoginMethod::parse("password"), None);
    }

    #[test]
    fn only_verified_non_relay_non_meta_email_matches() {
        assert!(email_usable_for_matching(&identity(
            LoginMethod::Apple,
            Some("a@example.test"),
            false
        )));
        assert!(email_usable_for_matching(&identity(
            LoginMethod::Google,
            Some("a@example.test"),
            false
        )));
        // Apple private relay: distinct identity, never string-matched.
        assert!(!email_usable_for_matching(&identity(
            LoginMethod::Apple,
            Some("xyz@privaterelay.appleid.com"),
            true
        )));
        // Meta-supplied email is never trusted for auto-linking.
        assert!(!email_usable_for_matching(&identity(
            LoginMethod::Meta,
            Some("a@example.test"),
            false
        )));
        // Unverified / empty emails never link.
        assert!(!email_usable_for_matching(&identity(
            LoginMethod::Google,
            None,
            false
        )));
        assert!(!email_usable_for_matching(&identity(
            LoginMethod::Google,
            Some(""),
            false
        )));
    }

    #[test]
    fn unknown_sub_never_auto_merges() {
        assert_eq!(
            decide_unknown_sub(true),
            NewIdentityDecision::PromptExplicitLink
        );
        assert_eq!(
            decide_unknown_sub(false),
            NewIdentityDecision::CreateFreshAccount
        );
    }

    #[test]
    fn link_and_unlink_gates() {
        assert!(explicit_link_permitted(true));
        assert!(!explicit_link_permitted(false));
        assert!(unlink_permitted(1, true));
        assert!(!unlink_permitted(0, true));
        assert!(!unlink_permitted(1, false));
    }

    #[test]
    fn revocation_signals_disable_and_kill() {
        for signal in [
            ProviderSignal::AppleConsentRevoked,
            ProviderSignal::AppleAccountDeleted,
            ProviderSignal::GoogleAccountDisabled,
            ProviderSignal::MetaDeauthorize,
            ProviderSignal::MetaDataDeletion,
        ] {
            let action = signal_action(signal);
            assert!(action.disable_link, "{signal:?}");
            assert!(action.kill_sessions, "{signal:?}");
        }
        let verify = signal_action(ProviderSignal::GoogleVerification);
        assert!(!verify.disable_link && !verify.kill_sessions);
        let toggled = signal_action(ProviderSignal::AppleEmailToggled);
        assert!(!toggled.disable_link && !toggled.kill_sessions);
        assert!(mint_magic_token().len() == 64);
        assert_ne!(token_hash("tok"), "tok");
        assert_eq!(MAGIC_LINK_TTL_SECS, 900);
    }
}
