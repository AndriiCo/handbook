//! Canonical Washington PO-Box fixtures (TEST-ONLY).
//!
//! The single home of the `40220`/`40234` values: every store test that
//! needs conflicting PO-Box facts uses [`PO_BOX_A`]/[`PO_BOX_B`] and
//! [`seed_wa_po_box_conflict`] instead of inventing values (see
//! `docs/specs/app-features.md`, "Tiny worked example"). The module is
//! `#[cfg(test)]`, so no production path can reference these values;
//! `db::tests::migrations_seed_no_facts_or_fixtures` proves no migration
//! seeds them either.

use rusqlite::Connection;

use contracts::Result;

use crate::db::{self, NewAuthority};
use crate::store::Store;

/// First canonical PO-Box statement (`fact#1` in the worked example).
pub const PO_BOX_A: &str = "PO Box 40220";

/// Second canonical PO-Box statement (`fact#2` in the worked example).
pub const PO_BOX_B: &str = "PO Box 40234";

/// Ids created by [`seed_wa_po_box_conflict`].
pub struct WaPoBoxFixture {
    pub authority_id: i64,
    pub capture_a: i64,
    pub capture_b: i64,
    pub fact_a: i64,
    pub fact_b: i64,
}

/// Seed the worked-example shape: one authority (`usa-wa-sos`), two
/// captures, and two `accepted` `po_box` facts with different statements.
/// Returns the new row ids. Runs on a migrated test connection; callers
/// own the transaction discipline (single-statement setup needs none).
pub fn seed_wa_po_box_conflict(conn: &Connection, store: &Store) -> Result<WaPoBoxFixture> {
    let authority_id = db::add_authority(
        conn,
        &NewAuthority {
            kind: "state",
            country: "USA",
            region: "Washington",
            name: "WA Secretary of State",
            slug: "usa-wa-sos",
            actor: "fixture",
            at: 1_800_000_000,
        },
    )?;
    let capture_a = insert_capture(conn, store, authority_id, PO_BOX_A)?;
    let capture_b = insert_capture(conn, store, authority_id, PO_BOX_B)?;
    let fact_a = insert_accepted_fact(conn, authority_id, capture_a, PO_BOX_A)?;
    let fact_b = insert_accepted_fact(conn, authority_id, capture_b, PO_BOX_B)?;
    Ok(WaPoBoxFixture {
        authority_id,
        capture_a,
        capture_b,
        fact_a,
        fact_b,
    })
}

/// Store `statement` as a blob and record its capture row.
fn insert_capture(
    conn: &Connection,
    store: &Store,
    authority_id: i64,
    statement: &str,
) -> Result<i64> {
    let sha = store.write_blob(statement.as_bytes())?;
    let rel = store
        .blob_path(&sha)?
        .strip_prefix(store.root())
        .expect("blob path is under the store root")
        .to_str()
        .expect("blob path is UTF-8")
        .to_string();
    conn.execute(
        "INSERT INTO capture(authority_id, captured_at, kind, path,
                             bytes_raw, bytes_stored, sha256, trust)
         VALUES (?1, 1_800_000_000, 'file', ?2, 0, 0, ?3, 'staff_verified')",
        rusqlite::params![authority_id, rel, sha],
    )?;
    Ok(conn.last_insert_rowid())
}

/// Record one `accepted` `po_box` fact citing `capture_id`.
fn insert_accepted_fact(
    conn: &Connection,
    authority_id: i64,
    capture_id: i64,
    statement: &str,
) -> Result<i64> {
    conn.execute(
        "INSERT INTO fact(authority_id, capture_id, topic, statement,
                         status, created_by, created_at)
         VALUES (?1, ?2, 'po_box', ?3, 'accepted', 'fixture', 1_800_000_000)",
        rusqlite::params![authority_id, capture_id, statement],
    )?;
    Ok(conn.last_insert_rowid())
}

/// The design §7 conflict query: same authority + topic, two `accepted`
/// facts with different statements. Returns `(fact_a_id, fact_b_id)` pairs.
fn conflicting_pairs(conn: &Connection) -> Result<Vec<(i64, i64)>> {
    let mut stmt = conn.prepare(
        "SELECT a.id, b.id FROM fact a JOIN fact b
           ON a.authority_id = b.authority_id AND a.topic = b.topic
          AND a.id < b.id
         WHERE a.status = 'accepted' AND b.status = 'accepted'
           AND a.statement != b.statement",
    )?;
    Ok(stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
        .collect::<rusqlite::Result<Vec<_>>>()?)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wa_po_box_fixture_flags_one_conflict_and_clears() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::new(dir.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();

        let fx = seed_wa_po_box_conflict(&conn, &store).unwrap();
        // The seeded rows link up: one authority, own capture per fact.
        let slug: String = conn
            .query_row(
                "SELECT slug FROM authority WHERE id = ?1",
                [fx.authority_id],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(slug, "usa-wa-sos");
        for (fact, capture) in [(fx.fact_a, fx.capture_a), (fx.fact_b, fx.capture_b)] {
            let cited: i64 = conn
                .query_row("SELECT capture_id FROM fact WHERE id = ?1", [fact], |row| {
                    row.get(0)
                })
                .unwrap();
            assert_eq!(cited, capture);
        }
        // Pilot proof: exactly one conflict pair, the two PO-Box facts.
        assert_eq!(conflicting_pairs(&conn).unwrap(), [(fx.fact_a, fx.fact_b)]);

        // Scripted resolution: reject #1 with a reason + audit row.
        conn.execute(
            "UPDATE fact SET status = 'rejected' WHERE id = ?1",
            [fx.fact_a],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
             VALUES (1_800_000_001, 'test', 'reject', 'fact', ?1,
                     'superseded by PO Box 40234 (capture #12)')",
            [fx.fact_a],
        )
        .unwrap();

        assert!(conflicting_pairs(&conn).unwrap().is_empty());
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit
                 WHERE action = 'reject' AND entity_id = ?1",
                [fx.fact_a],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(audits, 1);
    }
}
