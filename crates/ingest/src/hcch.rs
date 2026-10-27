//! F2 HCCH watch: Convention-membership status → routing derivation.
//!
//! A status row (member? since when?) derives exactly one routing:
//! member states take the apostille path, everyone else takes consular
//! legalization. The derived routing is stored as an S2-proposed fact
//! under the frozen `legalization_chain` topic (`status`/`routing` are
//! deliberately NOT fact topics — TOPICS.md is frozen).

/// Apostille Convention membership status for one country.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MemberStatus {
    /// Country name as the HCCH table spells it (e.g. `Viet Nam`).
    pub country: String,
    /// Listed as a contracting state.
    pub member: bool,
    /// Convention entry-into-force date (`YYYY-MM-DD`), if listed.
    pub effective: Option<String>,
}

/// Derived routing for outbound documents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Routing {
    Apostille,
    ConsularLegalization,
}

impl Routing {
    pub fn as_str(&self) -> &'static str {
        match self {
            Routing::Apostille => "apostille",
            Routing::ConsularLegalization => "consular legalization",
        }
    }
}

/// Derive routing from a membership status as of a date (`YYYY-MM-DD`).
///
/// Apostille only when the country is a member AND its listed effective
/// date is on or before `as_of`. Anything else — non-member, missing
/// or malformed effective date, effective date in the future — fails
/// closed to consular legalization. Dates compare lexicographically,
/// which orders well-formed `YYYY-MM-DD` values chronologically.
pub fn derive_routing(status: &MemberStatus, as_of: &str) -> Routing {
    let effective_ok = status
        .effective
        .as_deref()
        .is_some_and(|e| is_iso_date(e) && e <= as_of);
    if status.member && effective_ok {
        Routing::Apostille
    } else {
        Routing::ConsularLegalization
    }
}

/// One-line routing statement for a `legalization_chain` proposed fact.
pub fn routing_statement(status: &MemberStatus, routing: Routing) -> String {
    match (&status.effective, routing) {
        (Some(effective), Routing::Apostille) => {
            format!(
                "{}: {} (effective {effective})",
                status.country,
                routing.as_str()
            )
        }
        _ => format!("{}: {}", status.country, routing.as_str()),
    }
}

fn is_iso_date(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() == 10
        && bytes[4] == b'-'
        && bytes[7] == b'-'
        && bytes
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 4 && *i != 7)
            .all(|(_, b)| b.is_ascii_digit())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn viet_nam() -> MemberStatus {
        MemberStatus {
            country: "Viet Nam".to_string(),
            member: true,
            effective: Some("2026-09-11".to_string()),
        }
    }

    #[test]
    fn viet_nam_2026_09_11_routes_apostille() {
        assert_eq!(
            derive_routing(&viet_nam(), "2026-09-11"),
            Routing::Apostille
        );
        assert_eq!(
            routing_statement(&viet_nam(), Routing::Apostille),
            "Viet Nam: apostille (effective 2026-09-11)"
        );
    }

    #[test]
    fn routing_flips_on_the_effective_date() {
        // The 2026-10-13 misroute class: routing MUST follow the watched
        // status row, so the day before entry-into-force still routes
        // consular and the effective date itself routes apostille.
        assert_eq!(
            derive_routing(&viet_nam(), "2026-09-10"),
            Routing::ConsularLegalization
        );
        assert_eq!(
            derive_routing(&viet_nam(), "2026-09-11"),
            Routing::Apostille
        );
    }

    #[test]
    fn non_member_and_bad_dates_fail_closed_to_consular() {
        let non_member = MemberStatus {
            country: "Egypt".to_string(),
            member: false,
            effective: None,
        };
        assert_eq!(
            derive_routing(&non_member, "2026-09-11"),
            Routing::ConsularLegalization
        );
        for bad in [None, Some(""), Some("not-a-date"), Some("2026-13-99x")] {
            let status = MemberStatus {
                country: "Viet Nam".to_string(),
                member: true,
                effective: bad.map(str::to_string),
            };
            assert_eq!(
                derive_routing(&status, "2026-09-11"),
                Routing::ConsularLegalization,
                "effective={bad:?}"
            );
        }
        // Listed but not yet in force.
        let future = MemberStatus {
            country: "Viet Nam".to_string(),
            member: true,
            effective: Some("2027-01-01".to_string()),
        };
        assert_eq!(
            derive_routing(&future, "2026-09-11"),
            Routing::ConsularLegalization
        );
    }
}
