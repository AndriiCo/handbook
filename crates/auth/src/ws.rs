//! WS bus topics: parsing plus per-topic authorization.
//!
//! Topic nouns follow `docs/specs/README.md` (P2 nouns win;
//! `order`/`filing` retired). Anonymous callers may read
//! `authority:{slug}` only, and only accepted facts — enforced at
//! publish time by the bus, since anonymous subscribe itself is open.

use crate::Role;

/// One subscribable bus topic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WsTopic {
    /// Public handbook stream for one authority; accepted facts only.
    Authority { slug: String },
    /// Live call console; staff-only. The id is the 32-hex call id
    /// minted at dial (`new_call_id` in the web crate), so any
    /// well-formed id parses — a dangling id simply never receives
    /// events.
    Call { id: String },
    /// Order submission; owner-or-staff.
    Submission { id: i64 },
    /// Outbound shipment; owner-or-staff.
    Shipment { id: i64 },
    /// Billing events; owner-or-staff (owner named in the topic).
    Billing { customer_ref: String },
    /// Live draft-run steps (`draft-runs/{id}`); staff-only. The run id
    /// is a server-allocated counter, so any positive integer parses —
    /// a dangling id simply never receives events.
    Run { id: u64 },
    /// New-proposal feed for the `/review` queue; staff-only. One
    /// channel for all authorities — the event names its own.
    Proposals,
}

impl WsTopic {
    /// Parse `authority:{slug}` / `call:{id}` / `submission:{id}` /
    /// `shipment:{id}` / `billing:{customer_ref}` / `run:{id}` /
    /// bare `proposals`. Anything else — unknown nouns, empty args,
    /// traversal-looking slugs — is `None`.
    pub fn parse(input: &str) -> Option<Self> {
        if input == "proposals" {
            return Some(WsTopic::Proposals);
        }
        let (noun, arg) = input.split_once(':')?;
        if arg.is_empty() || arg.len() > 128 {
            return None;
        }
        match noun {
            "authority" if valid_slug(arg) => Some(WsTopic::Authority {
                slug: arg.to_string(),
            }),
            "call" if valid_call_id(arg) => Some(WsTopic::Call {
                id: arg.to_string(),
            }),
            "submission" => positive_id(arg).map(|id| WsTopic::Submission { id }),
            "shipment" => positive_id(arg).map(|id| WsTopic::Shipment { id }),
            "billing" if valid_customer_ref(arg) => Some(WsTopic::Billing {
                customer_ref: arg.to_string(),
            }),
            "run" => arg
                .parse::<u64>()
                .ok()
                .filter(|id| *id > 0)
                .map(|id| WsTopic::Run { id }),
            _ => None,
        }
    }

    /// Canonical channel name (what the bus keys broadcasts on).
    pub fn as_channel(&self) -> String {
        match self {
            WsTopic::Authority { slug } => format!("authority:{slug}"),
            WsTopic::Call { id } => format!("call:{id}"),
            WsTopic::Submission { id } => format!("submission:{id}"),
            WsTopic::Shipment { id } => format!("shipment:{id}"),
            WsTopic::Billing { customer_ref } => format!("billing:{customer_ref}"),
            WsTopic::Run { id } => format!("run:{id}"),
            WsTopic::Proposals => "proposals".to_string(),
        }
    }
}

/// Lowercase slug alphabet: no traversal, no case tricks.
fn valid_slug(slug: &str) -> bool {
    !slug.is_empty()
        && slug.len() <= 128
        && slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn valid_customer_ref(customer_ref: &str) -> bool {
    !customer_ref.is_empty()
        && customer_ref.len() <= 64
        && customer_ref
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}

fn positive_id(arg: &str) -> Option<i64> {
    arg.parse::<i64>().ok().filter(|id| *id > 0)
}

/// 32 lowercase-or-upper hex chars: the dial-minted call id alphabet.
fn valid_call_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Ownership lookup for `submission:{id}` / `shipment:{id}` topics.
///
/// SEAM (S6 owns the tables): until the P2 store lands, the web crate
/// passes [`NoOwners`], so customers fail closed on these topics and
/// only staff subscribe. S6 swaps in the real lookup; no authz
/// code changes.
pub trait OwnerLookup {
    /// Owning `customer_ref` of a submission, if any.
    fn submission_owner(&self, id: i64) -> Option<String>;
    /// Owning `customer_ref` of a shipment, if any.
    fn shipment_owner(&self, id: i64) -> Option<String>;
}

/// Pre-P2 lookup: no owner is ever known, customers fail closed.
pub struct NoOwners;

impl OwnerLookup for NoOwners {
    fn submission_owner(&self, _id: i64) -> Option<String> {
        None
    }

    fn shipment_owner(&self, _id: i64) -> Option<String> {
        None
    }
}

/// True when `role` (+ owned `customer_ref`) may subscribe to `topic`.
pub fn authorize(
    role: Role,
    customer_ref: Option<&str>,
    owners: &dyn OwnerLookup,
    topic: &WsTopic,
) -> bool {
    match topic {
        // Public read; accepted-only is enforced at publish.
        WsTopic::Authority { .. } => true,
        WsTopic::Call { .. } => role.meets(Role::Staff),
        WsTopic::Submission { id } => {
            role.meets(Role::Staff) || owns(role, customer_ref, owners.submission_owner(*id))
        }
        WsTopic::Shipment { id } => {
            role.meets(Role::Staff) || owns(role, customer_ref, owners.shipment_owner(*id))
        }
        WsTopic::Billing {
            customer_ref: topic_ref,
        } => {
            role.meets(Role::Staff)
                || (role.meets(Role::Customer) && customer_ref == Some(topic_ref.as_str()))
        }
        // Draft-run steps name no customer data, but run ids are
        // unguessable only by obscurity — staff-only, never anonymous.
        WsTopic::Run { .. } => role.meets(Role::Staff),
        // Proposal events name authority + statement text (unpublished
        // drafts) — staff-only, never anonymous.
        WsTopic::Proposals => role.meets(Role::Staff),
    }
}

/// Customer-owned check: the caller must be a logged-in customer
/// whose ref equals the looked-up owner.
fn owns(role: Role, customer_ref: Option<&str>, owner: Option<String>) -> bool {
    role.meets(Role::Customer) && matches!((customer_ref, owner), (Some(a), Some(b)) if a == b)
}

#[cfg(test)]
mod tests {
    use super::*;

    struct OneOwner;

    impl OwnerLookup for OneOwner {
        fn submission_owner(&self, id: i64) -> Option<String> {
            (id == 7).then(|| "cust-1".to_string())
        }

        fn shipment_owner(&self, _id: i64) -> Option<String> {
            None
        }
    }

    #[test]
    fn topics_parse_and_roundtrip() {
        for (text, expected) in [
            (
                "authority:usa-wa-sos",
                WsTopic::Authority {
                    slug: "usa-wa-sos".to_string(),
                },
            ),
            (
                "call:0123456789abcdef0123456789abcdef",
                WsTopic::Call {
                    id: "0123456789abcdef0123456789abcdef".to_string(),
                },
            ),
            ("submission:7", WsTopic::Submission { id: 7 }),
            ("shipment:3", WsTopic::Shipment { id: 3 }),
            (
                "billing:cust-1",
                WsTopic::Billing {
                    customer_ref: "cust-1".to_string(),
                },
            ),
            ("run:7", WsTopic::Run { id: 7 }),
            ("proposals", WsTopic::Proposals),
        ] {
            let parsed = WsTopic::parse(text).expect("parse");
            assert_eq!(parsed, expected);
            assert_eq!(parsed.as_channel(), text);
        }
    }

    #[test]
    fn topics_reject_garbage() {
        for bad in [
            "",
            "authority:",
            "authority:",
            "order:1",
            "filing:2",
            "authority:USA-WA",
            "authority:../etc",
            "authority:a/b",
            "call:0",
            "call:-1",
            "call:12",
            "call:abc",
            "call:1:2",
            "call:0123456789abcdef0123456789abcde",
            "call:0123456789abcdef0123456789abcdef0",
            "call:0123456789abcdef0123456789abcdeg",
            "submission:",
            "billing:",
            "billing:a b",
            "Billing:cust-1",
            "authority",
            "run:",
            "run:0",
            "run:-1",
            "run:abc",
            "Run:7",
            "proposals:x",
            "Proposals",
            "proposals ",
        ] {
            assert_eq!(WsTopic::parse(bad), None, "input: {bad:?}");
        }
    }

    #[test]
    fn anonymous_gets_authority_only() {
        let owners = NoOwners;
        let authority = WsTopic::parse("authority:usa-wa-sos").expect("parse");
        assert!(authorize(Role::Anonymous, None, &owners, &authority));
        for text in [
            "call:0123456789abcdef0123456789abcdef",
            "submission:7",
            "shipment:3",
            "billing:cust-1",
            "run:7",
            "proposals",
        ] {
            let topic = WsTopic::parse(text).expect("parse");
            assert!(
                !authorize(Role::Anonymous, None, &owners, &topic),
                "topic: {text}"
            );
        }
    }

    #[test]
    fn staff_subscribes_everywhere() {
        let owners = NoOwners;
        for text in [
            "authority:usa-wa-sos",
            "call:0123456789abcdef0123456789abcdef",
            "submission:7",
            "shipment:3",
            "billing:cust-9",
            "run:7",
            "proposals",
        ] {
            let topic = WsTopic::parse(text).expect("parse");
            assert!(authorize(Role::Staff, None, &owners, &topic));
            assert!(authorize(Role::Admin, None, &owners, &topic));
        }
        let call = WsTopic::parse("call:0123456789abcdef0123456789abcdef").expect("parse");
        assert!(!authorize(Role::Customer, Some("cust-1"), &owners, &call));
    }

    #[test]
    fn customers_need_ownership() {
        let none = NoOwners;
        let owned = OneOwner;
        let submission = WsTopic::parse("submission:7").expect("parse");
        // No lookup yet: fail closed even for the real owner.
        assert!(!authorize(
            Role::Customer,
            Some("cust-1"),
            &none,
            &submission
        ));
        // With the S6 lookup: owner in, others out.
        assert!(authorize(
            Role::Customer,
            Some("cust-1"),
            &owned,
            &submission
        ));
        assert!(!authorize(
            Role::Customer,
            Some("cust-2"),
            &owned,
            &submission
        ));
        // Billing names its owner, so no lookup is needed.
        let billing = WsTopic::parse("billing:cust-1").expect("parse");
        assert!(authorize(Role::Customer, Some("cust-1"), &none, &billing));
        assert!(!authorize(Role::Customer, Some("cust-2"), &none, &billing));
        assert!(!authorize(Role::Customer, None, &none, &billing));
        // Draft-run steps are staff-only even for logged-in customers.
        let run = WsTopic::parse("run:7").expect("parse");
        assert!(!authorize(Role::Customer, Some("cust-1"), &none, &run));
        // So is the proposals feed.
        let proposals = WsTopic::parse("proposals").expect("parse");
        assert!(!authorize(
            Role::Customer,
            Some("cust-1"),
            &none,
            &proposals
        ));
    }
}
