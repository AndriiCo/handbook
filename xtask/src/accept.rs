//! Deploy gate (roadmap G5): `xtask accept --expect-build <no>`.
//!
//! Verifies the store's latest `build` row is exactly the expected
//! build AND its `facts_hash` still matches a fresh recomputation
//! over today's accepted set. Success prints `ACCEPT ...` and writes
//! an `audit` row; any mismatch exits 1 with the reason. Tampering
//! with accepted facts after a build fails the gate.

/// Check the gate. `Ok(summary)` prints; `Err(reason)` exits 1.
pub fn accept(
    store: &handbook_store::store::Store,
    expect_build: i64,
    actor: &str,
) -> Result<String, String> {
    let conn = handbook_store::db::open(store).map_err(|e| e.to_string())?;
    let latest: Option<(i64, String)> = conn
        .query_row(
            "SELECT build_no, facts_hash FROM build ORDER BY build_no DESC LIMIT 1",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })
        .map_err(|e| e.to_string())?;
    let Some((build_no, stored)) = latest else {
        return Err("no build row: run `cargo xtask build-handbook` first".to_string());
    };
    if build_no != expect_build {
        return Err(format!(
            "latest build_no={build_no}, expected {expect_build}"
        ));
    }
    let fresh = publish::facts_hash(&conn).map_err(|e| e.to_string())?;
    if fresh != stored {
        return Err(format!(
            "facts_hash mismatch on build_no={build_no}: accepted set changed since the build"
        ));
    }
    let at = crate::unix_now();
    conn.execute(
        "INSERT INTO audit(at, actor, action, entity, entity_id, reason)
         VALUES (?1, ?2, 'accept', 'build', ?3, ?4)",
        rusqlite::params![
            at,
            actor,
            build_no,
            format!("build_no={build_no} facts_hash={stored}")
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(format!("ACCEPT build_no={build_no} facts_hash={stored}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn built() -> (tempfile::TempDir, handbook_store::store::Store) {
        let dir = tempfile::tempdir().unwrap();
        let store = handbook_store::store::Store::new(dir.path().join("store"));
        store.init().unwrap();
        let conn = handbook_store::db::open(&store).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        let wa = handbook_store::db::add_authority(
            &conn,
            &handbook_store::db::NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "WA",
                slug: "usa-wa-sos",
                actor: "test",
                at: 1,
            },
        )
        .unwrap();
        conn.execute(
            "INSERT INTO capture(authority_id, captured_at, kind, path,
                                 bytes_raw, bytes_stored, sha256, trust)
             VALUES (?1, 1, 'file', 'blob/0', 1, 1, ?2, 'staff_verified')",
            rusqlite::params![wa, "a".repeat(64)],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO fact(authority_id, capture_id, topic, statement,
                              status, created_by, created_at)
             VALUES (?1, 1, 'po_box', 'PO Box 40220', 'accepted', 'test', 1)",
            [wa],
        )
        .unwrap();
        publish::build_handbook(
            &conn,
            &store,
            &publish::BuildOptions {
                out_dir: dir.path().join("site"),
                base_url: "https://example.invalid".to_string(),
                actor: "test".to_string(),
                at: 1,
            },
        )
        .unwrap();
        (dir, store)
    }

    #[test]
    fn accept_passes_on_fresh_build() {
        let (_dir, store) = built();
        let out = accept(&store, 1, "test").unwrap();
        assert!(out.starts_with("ACCEPT"), "{out}");
    }

    #[test]
    fn accept_refuses_tamper_and_wrong_build() {
        let (_dir, store) = built();
        accept(&store, 2, "test").unwrap_err();
        let conn = handbook_store::db::open(&store).unwrap();
        conn.execute("UPDATE fact SET statement = 'PO Box 40221'", [])
            .unwrap();
        let err = accept(&store, 1, "test").unwrap_err();
        assert!(err.contains("mismatch"), "{err}");
    }
}
