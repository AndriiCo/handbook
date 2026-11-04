//! Field-level conflict reads (plan §5.1): which live siblings of one
//! fact disagree with it. The rule itself is
//! [`handbook_store::db::facts_disagree`], shared with the publish
//! build and the JS twin (`web/src/lib/public-ui/compare.js`); the
//! golden cases in `docs/contracts/conflict-cases.json` pin both.

use handbook_store::db;
use rusqlite::Connection;

use crate::approval::ConflictView;

/// Live siblings of one fact that disagree with it: other facts of
/// `authority_id` in its compared topics ([`db::compared_topics`]),
/// `accepted` or (unless `accepted_only`) `proposed`. Every candidate
/// is read and filtered in Rust before any caller caps the list, and
/// accepted siblings sort first (then by id), so a capped list keeps
/// the published ones.
pub(crate) fn disagreeing_siblings(
    conn: &Connection,
    authority_id: i64,
    topic: &str,
    statement: &str,
    exclude_id: i64,
    accepted_only: bool,
) -> rusqlite::Result<Vec<ConflictView>> {
    let mut stmt = conn.prepare(
        "SELECT id, topic, statement, status FROM fact
         WHERE authority_id = ?1 AND topic = ?2 AND id != ?3
           AND status IN ('accepted', 'proposed')",
    )?;
    let mut found = Vec::new();
    for sibling_topic in db::compared_topics(topic, statement) {
        let rows = stmt.query_map(
            rusqlite::params![authority_id, sibling_topic, exclude_id],
            |row| {
                Ok(ConflictView {
                    id: row.get(0)?,
                    topic: row.get(1)?,
                    statement: row.get(2)?,
                    status: row.get(3)?,
                })
            },
        )?;
        for row in rows {
            let sibling = row?;
            if (!accepted_only || sibling.status == "accepted")
                && db::facts_disagree(topic, statement, &sibling.topic, &sibling.statement)
            {
                found.push(sibling);
            }
        }
    }
    found.sort_by_key(|c| (c.status != "accepted", c.id));
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::store::Store;

    /// The golden cases `web/test/compare.test.mjs` also runs: the Rust
    /// rule answers every one as pinned, both ways round, and reads the
    /// pinned tokens where a case lists them.
    #[test]
    fn golden_vectors_match_the_shared_rule() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../../../docs/contracts/conflict-cases.json"))
                .unwrap();
        assert!(cases.len() >= 30, "the cases cover every rule");
        for case in &cases {
            let name = case["name"].as_str().unwrap();
            let topic = case["topic"].as_str().unwrap();
            let topic_b = case["topic_b"].as_str().unwrap_or(topic);
            let (a, b) = (case["a"].as_str().unwrap(), case["b"].as_str().unwrap());
            let want = case["disagree"].as_bool().unwrap();
            assert_eq!(
                db::facts_disagree(topic, a, topic_b, b),
                want,
                "case: {name}"
            );
            assert_eq!(
                db::facts_disagree(topic_b, b, topic, a),
                want,
                "case (reversed): {name}"
            );
            // One plain topic: the field rule alone decides.
            if topic_b == topic
                && !db::USPS_TOPICS.contains(&topic)
                && !db::NON_CONFLICTING_TOPICS.contains(&topic)
            {
                assert_eq!(
                    db::statements_disagree(a, b),
                    want,
                    "case (statements): {name}"
                );
            }
            for (side, statement) in [("tokens_a", a), ("tokens_b", b)] {
                let Some(pinned) = case[side].as_array() else {
                    continue;
                };
                let got: Vec<String> = db::value_tokens(db::split_statement(statement).1)
                    .into_iter()
                    .collect();
                let pinned: Vec<&str> = pinned.iter().map(|t| t.as_str().unwrap()).collect();
                assert_eq!(got, pinned, "{side}: {name}");
            }
        }
    }

    fn seeded() -> (tempfile::TempDir, Connection, i64, i64) {
        let tmp = tempfile::tempdir().unwrap();
        let store = Store::new(tmp.path());
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        let authority = db::add_authority(
            &conn,
            &db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "WA",
                name: "WA",
                slug: "usa-wa-sos",
                actor: "t",
                at: 1,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 1, 'file', 'p', 0, 0, 's', 'staff_verified')",
            [authority],
        )
        .unwrap();
        let capture = conn.last_insert_rowid();
        (tmp, conn, authority, capture)
    }

    fn add(
        conn: &Connection,
        authority: i64,
        capture: i64,
        topic: &str,
        statement: &str,
        status: &str,
    ) -> i64 {
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement,
                              status, created_by, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 't', 1)",
            rusqlite::params![authority, capture, topic, statement, status],
        )
        .unwrap();
        conn.last_insert_rowid()
    }

    #[test]
    fn conflict_filter_runs_before_the_cap() {
        let (_tmp, conn, authority, capture) = seeded();
        // Twelve other fields of the topic come first in id order.
        for i in 0..12 {
            add(
                &conn,
                authority,
                capture,
                "fees",
                &format!("field_{i}: ${i}"),
                "accepted",
            );
        }
        let published = add(
            &conn,
            authority,
            capture,
            "fees",
            "per_document: $15",
            "accepted",
        );
        let draft = add(
            &conn,
            authority,
            capture,
            "fees",
            "per_document: $20",
            "proposed",
        );
        let found = crate::approval::sibling_conflicts(
            &conn,
            authority,
            "fees",
            "per_document: $20",
            draft,
        );
        assert_eq!(found.iter().map(|c| c.id).collect::<Vec<_>>(), [published]);
        assert_eq!(found[0].topic, "fees");
    }

    #[test]
    fn conflict_cap_keeps_published_siblings_first() {
        let (tmp, conn, authority, capture) = seeded();
        let fact = add(
            &conn,
            authority,
            capture,
            "fees",
            "per_document: $15",
            "accepted",
        );
        let drafts: Vec<i64> = (0..11)
            .map(|i| {
                add(
                    &conn,
                    authority,
                    capture,
                    "fees",
                    &format!("per_document: ${}", 30 + i),
                    "proposed",
                )
            })
            .collect();
        let published = add(
            &conn,
            authority,
            capture,
            "fees",
            "per_document: $99",
            "accepted",
        );
        let found =
            crate::approval::sibling_conflicts(&conn, authority, "fees", "per_document: $15", fact);
        assert_eq!(found.len(), 10);
        assert_eq!(found[0].id, published);
        assert_eq!(
            found[1..].iter().map(|c| c.id).collect::<Vec<_>>(),
            drafts[..9]
        );
        // The public read still banners the published disagreement.
        let views = crate::approval::accepted_fact_views(&conn, tmp.path(), "usa-wa-sos").unwrap();
        let view = views.iter().find(|v| v.id == fact).unwrap();
        assert!(view.conflict);
        assert_eq!(
            view.conflicts.iter().map(|c| c.id).collect::<Vec<_>>(),
            [published]
        );
    }

    #[test]
    fn conflict_box_disputes_cross_the_usps_topics() {
        let (_tmp, conn, authority, capture) = seeded();
        let po_box = add(
            &conn,
            authority,
            capture,
            "po_box",
            "PO Box 40220",
            "accepted",
        );
        let courier = "courier: 243 Example Rd SE, Tumwater, WA 98501";
        let courier_id = add(&conn, authority, capture, "mailing", courier, "accepted");
        let mailing = "mailing_address: PO Box 40234, Olympia, WA 98504-0234";
        let draft = add(&conn, authority, capture, "mailing", mailing, "proposed");
        let found = crate::approval::sibling_conflicts(&conn, authority, "mailing", mailing, draft);
        assert_eq!(
            found
                .iter()
                .map(|c| (c.id, c.topic.as_str()))
                .collect::<Vec<_>>(),
            [(po_box, "po_box")]
        );
        // A courier street names no box: never compared with one.
        assert!(
            crate::approval::sibling_conflicts(&conn, authority, "mailing", courier, courier_id)
                .is_empty()
        );
        // Notices stack instead of conflicting.
        let notice = add(
            &conn,
            authority,
            capture,
            "service_notice",
            "notice: Closed Nov 2",
            "accepted",
        );
        add(
            &conn,
            authority,
            capture,
            "service_notice",
            "notice: Closed Nov 11",
            "accepted",
        );
        assert!(
            crate::approval::sibling_conflicts(
                &conn,
                authority,
                "service_notice",
                "notice: Closed Nov 2",
                notice
            )
            .is_empty()
        );
    }
}
