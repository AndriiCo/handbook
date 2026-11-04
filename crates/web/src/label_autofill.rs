//! Label autofill for staff `/labels` (P416): ship-to from an
//! authority's accepted facts, ship-from from the office
//! (`orders.ship_from`) or from a customer order's return address.
//!
//! Grounded only: every address and phone comes from an accepted fact
//! (statement or citation quote) and carries its `fact_id` and quote,
//! or from the order/config the operator entered. Nothing is guessed;
//! with no verified US street address the answer says so. A PO box is
//! returned as USPS-only (FedEx cannot deliver to one).
//!
//! Mounted on the fedex-live staff router (admin IP, host, session and
//! CSRF guards).

use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use handbook_store::{db, store::Store};
use rusqlite::OptionalExtension;
use serde_json::{Value, json};

use crate::{
    AppState,
    admin::AdminAuth,
    orders::{FactText, ReturnTo, UsAddress, us_address_in},
};

/// FedEx `personName`/`companyName` limit.
const NAME_MAX: usize = 35;

/// Street-type words that end a street (USPS Pub. 28 common suffixes).
const SUFFIXES: &[&str] = &[
    "st",
    "street",
    "rd",
    "road",
    "ave",
    "av",
    "avenue",
    "blvd",
    "boulevard",
    "dr",
    "drive",
    "ln",
    "lane",
    "way",
    "ct",
    "court",
    "pl",
    "place",
    "pkwy",
    "parkway",
    "hwy",
    "highway",
    "cir",
    "circle",
    "ter",
    "terrace",
    "trl",
    "trail",
    "sq",
    "square",
    "loop",
    "plz",
    "plaza",
    "aly",
    "alley",
    "row",
    "walk",
    "pike",
    "expy",
    "fwy",
];
const DIRECTIONALS: &[&str] = &["n", "s", "e", "w", "ne", "nw", "se", "sw"];

/// One US address found in a fact, with the building or suite words
/// written before the street number (`Town Center 3`) as line 2.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FoundAddress {
    pub address: UsAddress,
    pub line2: Option<String>,
}

/// The first US address in `text`: the strict `street, City, ST 12345`
/// form first, then `street City, ST 12345` where the street ends at a
/// street-type word (plus a directional) or a PO box number.
pub fn address_in(text: &str) -> Option<FoundAddress> {
    for line in text.lines() {
        if let Some(address) = us_address_in(line) {
            return Some(FoundAddress {
                address,
                line2: None,
            });
        }
        if let Some(found) = loose_address_in(line) {
            return Some(found);
        }
    }
    None
}

fn is_zip(s: &str) -> bool {
    let b = s.as_bytes();
    (b.len() == 5 && b.iter().all(u8::is_ascii_digit))
        || (b.len() == 10
            && b[..5].iter().all(u8::is_ascii_digit)
            && b[5] == b'-'
            && b[6..].iter().all(u8::is_ascii_digit))
}

fn loose_address_in(line: &str) -> Option<FoundAddress> {
    let (head, tail) = line.rsplit_once(',')?;
    let mut words = tail.split_whitespace();
    let state = words.next()?;
    let zip = words.next()?.trim_end_matches(['.', ';', ')']);
    if state.len() != 2 || !state.bytes().all(|b| b.is_ascii_uppercase()) || !is_zip(zip) {
        return None;
    }
    // Drop a `key:` label (`apostille_mailing_address: PO BOX …`).
    let head = head.rsplit(':').next().unwrap_or(head).trim();
    let tokens: Vec<&str> = head.split_whitespace().collect();
    let lower: Vec<String> = tokens
        .iter()
        .map(|t| {
            t.trim_matches(|c: char| !c.is_ascii_alphanumeric())
                .to_ascii_lowercase()
        })
        .collect();
    let numeric = |i: usize| lower[i].bytes().all(|b| b.is_ascii_digit()) && !lower[i].is_empty();
    // PO box: `PO BOX 40228 City` / `P.O. Box 40228 City`.
    if let Some(i) = (0..tokens.len().saturating_sub(2)).find(|&i| {
        (lower[i] == "po" && lower[i + 1] == "box") || (lower[i] == "p" && lower[i + 1] == "o")
    }) {
        let box_at = if lower[i + 1] == "box" { i + 2 } else { i + 3 };
        if box_at < tokens.len() && numeric(box_at) && box_at + 1 < tokens.len() {
            let city = tokens[box_at + 1..].join(" ");
            return Some(FoundAddress {
                address: UsAddress {
                    street: format!("PO Box {}", tokens[box_at]),
                    city,
                    state: state.to_string(),
                    zip: zip.to_string(),
                },
                line2: None,
            });
        }
        return None;
    }
    // Street: the last number before the first street-type word.
    let suffix = (1..tokens.len()).find(|&i| SUFFIXES.contains(&lower[i].as_str()))?;
    let start = (0..suffix).rev().find(|&i| numeric(i))?;
    let mut end = suffix + 1;
    if end < tokens.len() && DIRECTIONALS.contains(&lower[end].as_str()) {
        end += 1;
    }
    if end >= tokens.len() {
        return None;
    }
    let city = tokens[end..].join(" ");
    let street = tokens[start..end].join(" ");
    let line2 = (start > 0).then(|| tokens[..start].join(" "));
    if city.len() > 40 || street.len() > 80 {
        return None;
    }
    Some(FoundAddress {
        address: UsAddress {
            street,
            city,
            state: state.to_string(),
            zip: zip.to_string(),
        },
        line2,
    })
}

/// The first 10-digit US phone number in `text`, as `NNN-NNN-NNNN`.
pub fn phone_in(text: &str) -> Option<String> {
    let bytes = text.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() || bytes[i] == b'(' || bytes[i] == b'+' {
            let mut digits = String::new();
            let mut j = i;
            while j < bytes.len()
                && (bytes[j].is_ascii_digit() || b"()-. +".contains(&bytes[j]))
                && digits.len() < 11
            {
                if bytes[j].is_ascii_digit() {
                    digits.push(bytes[j] as char);
                }
                j += 1;
            }
            let digits = digits
                .strip_prefix('1')
                .filter(|d| d.len() == 10)
                .map_or(digits.clone(), str::to_string);
            if digits.len() == 10 {
                return Some(format!(
                    "{}-{}-{}",
                    &digits[..3],
                    &digits[3..6],
                    &digits[6..]
                ));
            }
            i = j.max(i + 1);
        } else {
            i += 1;
        }
    }
    None
}

/// `name` clamped to FedEx's 35 characters at a word boundary.
pub fn label_name(name: &str) -> String {
    let name = name.split(" — ").next().unwrap_or(name).trim();
    if name.chars().count() <= NAME_MAX {
        return name.to_string();
    }
    let mut out = String::new();
    for word in name.split_whitespace() {
        if out.chars().count() + word.chars().count() + usize::from(!out.is_empty()) > NAME_MAX {
            break;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
    }
    out
}

fn excerpt(text: &str) -> String {
    let first = text.lines().next().unwrap_or(text).trim();
    first.chars().take(200).collect()
}

/// Ship-to candidates for one authority from its accepted facts:
/// every US address (street first, PO boxes marked USPS-only), the
/// first phone, and the recommended FedEx address (the first street
/// address in `mailing`, then `address`, then `po_box`/other order).
pub fn authority_fill(slug: &str, name: &str, facts: &[FactText]) -> Value {
    let rank = |t: &str| match t {
        "mailing" => 0,
        "address" => 1,
        "po_box" => 2,
        _ => 3,
    };
    let mut ordered: Vec<&FactText> = facts
        .iter()
        .filter(|f| {
            matches!(
                f.topic.as_str(),
                "mailing" | "address" | "po_box" | "dropbox"
            )
        })
        .collect();
    ordered.sort_by_key(|f| (rank(&f.topic), f.id));
    let mut addresses = Vec::new();
    for fact in ordered {
        let Some(found) = address_in(&fact.text) else {
            continue;
        };
        let po_box = found.address.is_po_box();
        let label = fact
            .text
            .split(':')
            .next()
            .filter(|k| k.len() < 60 && !k.contains(' '))
            .map(|k| k.replace('_', " "))
            .unwrap_or_else(|| fact.topic.replace('_', " "));
        addresses.push(json!({
            "fact_id": fact.id,
            "topic": fact.topic,
            "label": label,
            "street1": found.address.street,
            "street2": found.line2,
            "city": found.address.city,
            "state_province": found.address.state,
            "postal": found.address.zip,
            "country": "US",
            "po_box": po_box,
            "fedex_ok": !po_box,
            "quote": excerpt(&fact.text),
        }));
    }
    let recommended = addresses.iter().position(|a| a["fedex_ok"] == true);
    let phone = facts
        .iter()
        .filter(|f| matches!(f.topic.as_str(), "contacts" | "phone_script"))
        .chain(facts.iter())
        .find_map(|f| {
            phone_in(&f.text)
                .map(|p| json!({"value": p, "fact_id": f.id, "quote": excerpt(&f.text)}))
        });
    let reason = if addresses.is_empty() {
        Some("No staff-approved US address for this authority yet.")
    } else if recommended.is_none() {
        Some("Only a PO box is on file: FedEx cannot deliver to it (USPS only).")
    } else {
        None
    };
    json!({
        "authority": {"slug": slug, "name": name},
        "name": label_name(name),
        "addresses": addresses,
        "recommended": recommended,
        "phone": phone,
        "reason": reason,
    })
}

/// An office or customer return address as a label address.
pub fn return_address(r: &ReturnTo) -> Value {
    json!({
        "name": label_name(&r.name),
        "street1": r.line1,
        "street2": r.line2,
        "city": r.city,
        "state_province": r.region,
        "postal": r.postal_code,
        "country": r.country,
        "phone": r.phone,
    })
}

fn err(status: StatusCode, message: &str) -> Response {
    (status, Json(json!({ "error": message }))).into_response()
}

/// Why an authority lookup produced no fill.
enum Refusal {
    Store,
    Unknown,
}

impl Refusal {
    fn response(self) -> Response {
        match self {
            Refusal::Store => crate::store_unavailable(),
            Refusal::Unknown => err(StatusCode::NOT_FOUND, "unknown authority"),
        }
    }
}

fn authority_value(state: &AppState, slug: &str) -> Result<Value, Refusal> {
    let store = Store::new(&state.store_root);
    let conn = db::open(&store).map_err(|_| Refusal::Store)?;
    let name: Option<String> = conn
        .query_row(
            "SELECT name FROM authority WHERE slug = ?1",
            [slug],
            |row| row.get(0),
        )
        .optional()
        .map_err(|_| Refusal::Store)?;
    let name = name.ok_or(Refusal::Unknown)?;
    let facts = crate::orders::fact_texts(state, &conn, slug);
    Ok(authority_fill(slug, &name, &facts))
}

/// `GET /fedex-live/autofill/authority/{slug}`.
pub async fn authority(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Path(slug): Path<String>,
) -> Response {
    match authority_value(&state, slug.trim()) {
        Ok(value) => Json(value).into_response(),
        Err(refusal) => refusal.response(),
    }
}

/// `GET /fedex-live/autofill/office`: `orders.ship_from`.
pub async fn office(State(state): State<AppState>, _auth: AdminAuth) -> Response {
    let Ok(conn) = db::open(&Store::new(&state.store_root)) else {
        return crate::store_unavailable();
    };
    match crate::orders::ship_from(&state, &conn) {
        Some(from) => Json(json!({ "from": return_address(&from) })).into_response(),
        None => err(
            StatusCode::NOT_FOUND,
            "orders.ship_from is not set (or not a valid address) in /config",
        ),
    }
}

/// `GET /fedex-live/autofill/order/{id}`: the customer's return
/// address as ship-from and the order's authority as ship-to.
pub async fn order(
    State(state): State<AppState>,
    _auth: AdminAuth,
    Path(id): Path<String>,
) -> Response {
    let Ok(p2) = db::open_p2(&Store::new(&state.store_root)) else {
        return crate::store_unavailable();
    };
    let row: Option<(String, String, String)> = p2
        .query_row(
            "SELECT kind, authority_slug, return_json FROM customer_order WHERE id = ?1",
            [id.trim()],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .optional()
        .unwrap_or(None);
    let Some((kind, slug, return_json)) = row else {
        return err(StatusCode::NOT_FOUND, "unknown order");
    };
    let from = serde_json::from_str::<ReturnTo>(&return_json)
        .ok()
        .map(|r| return_address(&r));
    let to = match authority_value(&state, &slug) {
        Ok(value) => value,
        Err(refusal) => return refusal.response(),
    };
    Json(json!({
        "order_id": id.trim(),
        "kind": kind,
        "authority_slug": slug,
        "from": from,
        "to": to,
    }))
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(id: i64, topic: &str, text: &str) -> FactText {
        FactText {
            id,
            topic: topic.to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn reads_washington_sos_facts_as_written() {
        let main = address_in(
            "apostille_main_branch_street: Town Center 3 243 Israel Rd SE Tumwater, WA 98501",
        )
        .unwrap();
        assert_eq!(main.address.street, "243 Israel Rd SE");
        assert_eq!(main.address.city, "Tumwater");
        assert_eq!(main.address.state, "WA");
        assert_eq!(main.address.zip, "98501");
        assert_eq!(main.line2.as_deref(), Some("Town Center 3"));

        let cheney =
            address_in("apostille_eastern_branch_street: 960 Washington St Cheney, WA 99004")
                .unwrap();
        assert_eq!(cheney.address.street, "960 Washington St");
        assert_eq!(cheney.address.city, "Cheney");
        assert_eq!(cheney.line2, None);

        let po =
            address_in("apostille_mailing_address: PO BOX 40228 Olympia, WA 98504-0228").unwrap();
        assert_eq!(po.address.street, "PO Box 40228");
        assert_eq!(po.address.city, "Olympia");
        assert_eq!(po.address.zip, "98504-0228");
        assert!(po.address.is_po_box());
    }

    #[test]
    fn strict_form_still_wins_and_prose_is_ignored() {
        let strict = address_in("Mail to 801 Capitol Way S, Olympia, WA 98501.").unwrap();
        assert_eq!(strict.address.street, "801 Capitol Way S");
        assert_eq!(strict.address.city, "Olympia");
        assert_eq!(
            address_in("Send all documents with Request form and payment:"),
            None
        );
        assert_eq!(address_in("Open 8 to 5, Monday to Friday"), None);
    }

    #[test]
    fn phones_are_normalized() {
        assert_eq!(
            phone_in("apostille_phone: 360-725-0377").as_deref(),
            Some("360-725-0377")
        );
        assert_eq!(
            phone_in("Call (360) 725 0377 today").as_deref(),
            Some("360-725-0377")
        );
        assert_eq!(phone_in("+1 360.725.0377").as_deref(), Some("360-725-0377"));
        assert_eq!(phone_in("Fee $15, 3 days"), None);
    }

    #[test]
    fn authority_fill_recommends_a_street_and_marks_the_po_box() {
        let facts = vec![
            f(
                10,
                "address",
                "apostille_main_branch_street: Town Center 3 243 Israel Rd SE Tumwater, WA 98501",
            ),
            f(
                11,
                "mailing",
                "apostille_mailing_address: PO BOX 40228 Olympia, WA 98504-0228",
            ),
            f(12, "contacts", "apostille_phone: 360-725-0377"),
            f(
                13,
                "address",
                "apostille_eastern_branch_street: 960 Washington St Cheney, WA 99004",
            ),
            f(
                14,
                "mailing",
                "submission_requirement: Send all documents with Request form and payment:",
            ),
        ];
        let v = authority_fill(
            "usa-wa-sos",
            "Washington Secretary of State — Apostilles & Authentications",
            &facts,
        );
        assert_eq!(v["name"], "Washington Secretary of State");
        let addresses = v["addresses"].as_array().unwrap();
        assert_eq!(addresses.len(), 3);
        // Mailing (PO box) ranks first but FedEx skips it.
        assert_eq!(addresses[0]["po_box"], true);
        assert_eq!(addresses[0]["fedex_ok"], false);
        let pick = &addresses[v["recommended"].as_u64().unwrap() as usize];
        assert_eq!(pick["street1"], "243 Israel Rd SE");
        assert_eq!(pick["street2"], "Town Center 3");
        assert_eq!(pick["fact_id"], 10);
        assert_eq!(v["phone"]["value"], "360-725-0377");
        assert_eq!(v["phone"]["fact_id"], 12);
        assert!(v["reason"].is_null());
    }

    #[test]
    fn no_address_says_so() {
        let v = authority_fill("x", "X", &[f(1, "fees", "$15 per document")]);
        assert_eq!(v["addresses"].as_array().unwrap().len(), 0);
        assert!(v["recommended"].is_null());
        assert_eq!(
            v["reason"],
            "No staff-approved US address for this authority yet."
        );
        let po = authority_fill("y", "Y", &[f(2, "po_box", "PO Box 9, Salem, OR 97301")]);
        assert_eq!(
            po["reason"],
            "Only a PO box is on file: FedEx cannot deliver to it (USPS only)."
        );
    }

    #[test]
    fn names_fit_fedex() {
        assert_eq!(
            label_name("Texas Secretary of State — Authentications"),
            "Texas Secretary of State"
        );
        let long =
            label_name("Ministry of Foreign Affairs and International Cooperation of the Republic");
        assert!(long.chars().count() <= 35, "{long}");
    }
}
