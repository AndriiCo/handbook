//! `config-import`: seal many config values from one payload, in one
//! transaction, validated against the registry.
//!
//! ```text
//! cargo xtask config-import --store <dir> --kek-file <path>
//!     (--file <payload.json> | --stdin) [--mode merge|replace]
//!     [--dry-run] [--enroll-if-empty] [--actor <a>]
//! ```
//!
//! Payload: `{"format": "handbook-config-import/1", "values": {"net.port":
//! "18318", "quic.enabled": true, "meta.api_key": "..."}}`. Types come from
//! `config::keys::ALL`: bool keys take a JSON bool, text keys a string or
//! number, secret keys a string. Removed keys and `setup.*` are refused.
//! `replace` also deletes stored rows the payload does not name. The
//! import is checked by loading the runtime config inside the
//! transaction: any boot warning rolls everything back. Output names keys
//! and actions only, never values. `--enroll-if-empty` records the given
//! KEK as generation 1 on a store that has none (first install).

use std::collections::BTreeMap;

use sealed_config::{ConfigType, Kek, SetValue, keys};
use serde::Deserialize;
use serde_json::Value;

pub const FORMAT: &str = "handbook-config-import/1";

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Payload {
    pub format: String,
    pub values: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Merge,
    Replace,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Set,
    Unchanged,
    Delete,
}

/// One validated write: key, registry type, plaintext bytes.
pub struct Item {
    pub key: String,
    pub kind: ConfigType,
    pub plaintext: Vec<u8>,
}

/// Check every value against the registry and turn it into plaintext
/// bytes (bools as `1`/`0`, numbers as their decimal text).
pub fn validate(payload: &Payload) -> Result<Vec<Item>, String> {
    if payload.format != FORMAT {
        return Err(format!(
            "format must be {FORMAT:?}, got {:?}",
            payload.format
        ));
    }
    let mut out = Vec::with_capacity(payload.values.len());
    let mut problems = Vec::new();
    for (key, value) in &payload.values {
        if key.starts_with("setup.") {
            problems.push(format!(
                "{key}: setup keys are minted on the server, never imported"
            ));
            continue;
        }
        if keys::REMOVED.contains(&key.as_str()) {
            problems.push(format!(
                "{key}: removed from the registry; nothing reads it"
            ));
            continue;
        }
        let Some((_, kind)) = keys::ALL.iter().find(|(k, _)| k == key) else {
            problems.push(format!("{key}: not a registered config key"));
            continue;
        };
        let plaintext = match (kind, value) {
            (ConfigType::Bool, Value::Bool(b)) => vec![if *b { b'1' } else { b'0' }],
            (ConfigType::Text, Value::String(s)) => s.as_bytes().to_vec(),
            (ConfigType::Text, Value::Number(n)) => n.to_string().into_bytes(),
            (ConfigType::Secret, Value::String(s)) if !s.is_empty() => s.as_bytes().to_vec(),
            (kind, _) => {
                problems.push(format!("{key}: expected a {} value", kind.as_str()));
                continue;
            }
        };
        out.push(Item {
            key: key.clone(),
            kind: *kind,
            plaintext,
        });
    }
    if problems.is_empty() {
        Ok(out)
    } else {
        Err(problems.join("; "))
    }
}

/// The stored plaintext of `key` with its stored type, if any (the
/// `*_for_job` readers refuse a type mismatch, which reads as changed).
fn stored(conn: &rusqlite::Connection, kek: &Kek, key: &str, kind: ConfigType) -> Option<Vec<u8>> {
    match kind {
        ConfigType::Bool => sealed_config::get_bool_for_job(conn, kek, key)
            .ok()
            .flatten()
            .map(|b| vec![if b { b'1' } else { b'0' }]),
        ConfigType::Text => sealed_config::get_text_for_job(conn, kek, key)
            .ok()
            .flatten()
            .map(String::into_bytes),
        ConfigType::Secret => sealed_config::get_secret_for_job(conn, kek, key)
            .ok()
            .flatten(),
    }
}

/// What the import would do, key by key (sorted).
pub fn plan(
    conn: &rusqlite::Connection,
    kek: &Kek,
    items: &[Item],
    mode: Mode,
) -> Result<Vec<(String, Action)>, String> {
    let mut out: Vec<(String, Action)> = items
        .iter()
        .map(|item| {
            let same =
                stored(conn, kek, &item.key, item.kind).as_deref() == Some(&item.plaintext[..]);
            (
                item.key.clone(),
                if same { Action::Unchanged } else { Action::Set },
            )
        })
        .collect();
    if mode == Mode::Replace {
        let existing: Vec<String> = conn
            .prepare("SELECT key FROM config ORDER BY key")
            .and_then(|mut stmt| {
                stmt.query_map([], |r| r.get(0))?
                    .collect::<rusqlite::Result<Vec<String>>>()
            })
            .map_err(|e| e.to_string())?;
        for key in existing {
            // The setup gate is minted on the server; replace never wipes it.
            if !key.starts_with("setup.") && !items.iter().any(|i| i.key == key) {
                out.push((key, Action::Delete));
            }
        }
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

/// Apply a plan in one transaction, then prove the boot loader accepts
/// the result; any load error or warning rolls everything back.
pub fn apply(
    conn: &rusqlite::Connection,
    kek: &Kek,
    items: &[Item],
    plan: &[(String, Action)],
    actor: &str,
    now: i64,
) -> Result<usize, String> {
    let version = sealed_config::active_kek_version(conn)
        .map_err(|e| e.to_string())?
        .ok_or("no active KEK: enroll one first (or pass --enroll-if-empty)")?;
    conn.execute_batch("BEGIN IMMEDIATE")
        .map_err(|e| e.to_string())?;
    let result = (|| -> Result<usize, String> {
        let mut changed = 0;
        for (key, action) in plan {
            match action {
                Action::Unchanged => continue,
                Action::Set => {
                    let item = items
                        .iter()
                        .find(|i| &i.key == key)
                        .ok_or("plan out of step")?;
                    sealed_config::set_value(
                        conn,
                        kek,
                        &SetValue {
                            version,
                            key,
                            key_type: item.kind,
                            plaintext: &item.plaintext,
                            now,
                            actor,
                        },
                    )
                    .map_err(|e| format!("{key}: {e}"))?;
                }
                Action::Delete => {
                    sealed_config::delete_value(conn, key).map_err(|e| format!("{key}: {e}"))?;
                }
            }
            let verb = if *action == Action::Delete {
                "delete"
            } else {
                "set"
            };
            conn.execute(
                "INSERT INTO audit(at, actor, action, entity, entity_id, before_ref, after_ref, reason)
                 VALUES (?1, ?2, 'config_import', 'config', 0, ?3, ?4, '')",
                rusqlite::params![now, actor, key, verb],
            )
            .map_err(|e| e.to_string())?;
            changed += 1;
        }
        let (_, warnings) =
            sealed_config::RuntimeConfig::load(conn, kek).map_err(|e| format!("boot load: {e}"))?;
        if !warnings.is_empty() {
            return Err(format!(
                "the server would warn at boot: {}",
                warnings.join("; ")
            ));
        }
        Ok(changed)
    })();
    match result {
        Ok(n) => {
            conn.execute_batch("COMMIT").map_err(|e| e.to_string())?;
            sealed_config::verify_all(conn, kek).map_err(|e| format!("verify: {e}"))?;
            Ok(n)
        }
        Err(e) => {
            let _ = conn.execute_batch("ROLLBACK");
            Err(e)
        }
    }
}

fn fail(message: &str) -> ! {
    eprintln!("config-import: {message}");
    std::process::exit(1);
}

pub fn run(args: &[String]) {
    let store = crate::open_store(args);
    let conn = handbook_store::db::open(&store).unwrap_or_else(|e| fail(&e.to_string()));
    handbook_store::db::migrate(&conn).unwrap_or_else(|e| fail(&e.to_string()));
    sealed_config::migrate_config_tables(&conn).unwrap_or_else(|e| fail(&e.to_string()));
    let kek_path = crate::flag(args, "kek-file")
        .or_else(|| std::env::var("HANDBOOK_KEK_FILE").ok())
        .unwrap_or_else(|| fail("missing --kek-file (or HANDBOOK_KEK_FILE)"));
    let bytes = std::fs::read(&kek_path).unwrap_or_else(|e| fail(&format!("read KEK: {e}")));
    let raw: [u8; 32] = bytes
        .as_slice()
        .try_into()
        .unwrap_or_else(|_| fail("KEK must be exactly 32 bytes"));
    let kek = Kek::from_bytes(raw);
    let text = if args.iter().any(|a| a == "--stdin") {
        use std::io::Read as _;
        let mut buf = String::new();
        std::io::stdin()
            .read_to_string(&mut buf)
            .unwrap_or_else(|e| fail(&e.to_string()));
        buf
    } else {
        let path =
            crate::flag(args, "file").unwrap_or_else(|| fail("pass --file <path> or --stdin"));
        std::fs::read_to_string(&path).unwrap_or_else(|e| fail(&format!("read {path}: {e}")))
    };
    let payload: Payload =
        serde_json::from_str(&text).unwrap_or_else(|e| fail(&format!("payload: {e}")));
    let mode = match crate::flag(args, "mode").as_deref() {
        None | Some("merge") => Mode::Merge,
        Some("replace") => Mode::Replace,
        Some(other) => fail(&format!("--mode must be merge or replace, not {other}")),
    };
    let items = validate(&payload).unwrap_or_else(|e| fail(&e));
    let now = crate::unix_now();
    if sealed_config::active_kek_version(&conn)
        .unwrap_or_else(|e| fail(&e.to_string()))
        .is_none()
    {
        if args.iter().any(|a| a == "--enroll-if-empty") && !args.iter().any(|a| a == "--dry-run") {
            sealed_config::record_kek(&conn, 1, "", now).unwrap_or_else(|e| fail(&e.to_string()));
            println!("enrolled the given KEK as generation 1");
        } else if !args.iter().any(|a| a == "--dry-run") {
            fail("no active KEK in this store (first install: pass --enroll-if-empty)");
        }
    }
    let plan = plan(&conn, &kek, &items, mode).unwrap_or_else(|e| fail(&e));
    for (key, action) in &plan {
        let word = match action {
            Action::Set => "set",
            Action::Unchanged => "unchanged",
            Action::Delete => "delete",
        };
        println!("{word:<9} {key}");
    }
    if args.iter().any(|a| a == "--dry-run") {
        println!("dry run: nothing written");
        return;
    }
    let actor = crate::flag(args, "actor").unwrap_or_else(|| "config-import".to_string());
    match apply(&conn, &kek, &items, &plan, &actor, now) {
        Ok(n) => println!(
            "imported: {n} change(s), {} key(s) in the payload",
            items.len()
        ),
        Err(e) => fail(&format!("{e}; nothing was written")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> (tempfile::TempDir, rusqlite::Connection, Kek) {
        let dir = tempfile::tempdir().unwrap();
        let s = handbook_store::store::Store::new(dir.path());
        s.init().unwrap();
        let conn = handbook_store::db::open(&s).unwrap();
        handbook_store::db::migrate(&conn).unwrap();
        sealed_config::migrate_config_tables(&conn).unwrap();
        sealed_config::record_kek(&conn, 1, "", 1).unwrap();
        (dir, conn, Kek::from_bytes([7u8; 32]))
    }

    fn payload(values: serde_json::Value) -> Payload {
        serde_json::from_value(serde_json::json!({"format": FORMAT, "values": values})).unwrap()
    }

    #[test]
    fn values_are_typed_by_the_registry() {
        let items = validate(&payload(serde_json::json!({
            "net.port": 18318, "quic.enabled": true, "meta.api_key": "LLM_1_synthetic",
        })))
        .unwrap();
        assert_eq!(items.len(), 3);
        let port = items.iter().find(|i| i.key == "net.port").unwrap();
        assert_eq!(port.plaintext, b"18318");
        let quic = items.iter().find(|i| i.key == "quic.enabled").unwrap();
        assert_eq!(quic.plaintext, b"1");
        let err = validate(&payload(serde_json::json!({
            "quic.enabled": "yes", "no.such": "x", "tls.client_ca_file": "/x",
            "setup.key_hash": "x", "meta.api_key": "",
        })))
        .err()
        .expect("invalid payload");
        for needle in [
            "quic.enabled",
            "no.such",
            "tls.client_ca_file",
            "setup.key_hash",
            "meta.api_key",
        ] {
            assert!(err.contains(needle), "{needle}: {err}");
        }
        let wrong = Payload {
            format: "other/1".into(),
            values: BTreeMap::new(),
        };
        assert!(validate(&wrong).is_err());
    }

    #[test]
    fn merge_then_replace_with_audit_and_unchanged_detection() {
        let (_d, conn, kek) = store();
        let first = validate(&payload(serde_json::json!({
            "net.port": "18318", "net.admin_port": "18319", "meta.api_key": "LLM_1_synthetic",
        })))
        .unwrap();
        let p = plan(&conn, &kek, &first, Mode::Merge).unwrap();
        assert!(p.iter().all(|(_, a)| *a == Action::Set));
        assert_eq!(apply(&conn, &kek, &first, &p, "test", 10).unwrap(), 3);
        // Same values again: nothing changes.
        let p = plan(&conn, &kek, &first, Mode::Merge).unwrap();
        assert!(p.iter().all(|(_, a)| *a == Action::Unchanged), "{p:?}");
        assert_eq!(apply(&conn, &kek, &first, &p, "test", 11).unwrap(), 0);
        // Replace with fewer keys: the rest is deleted.
        let second = validate(&payload(serde_json::json!({"net.port": "18400"}))).unwrap();
        let p = plan(&conn, &kek, &second, Mode::Replace).unwrap();
        assert!(p.contains(&("net.port".to_string(), Action::Set)));
        assert!(p.contains(&("net.admin_port".to_string(), Action::Delete)));
        assert!(p.contains(&("meta.api_key".to_string(), Action::Delete)));
        apply(&conn, &kek, &second, &p, "test", 12).unwrap();
        let left: i64 = conn
            .query_row("SELECT COUNT(*) FROM config", [], |r| r.get(0))
            .unwrap();
        assert_eq!(left, 1);
        let audits: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM audit WHERE action = 'config_import'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(audits, 3 + 3);
    }

    #[test]
    fn a_boot_warning_rolls_the_whole_import_back() {
        let (_d, conn, kek) = store();
        // An unparsable allowlist makes the boot loader warn.
        let bad = validate(&payload(serde_json::json!({
            "net.port": "18318", "admin.allow_ips": "not an ip range",
        })))
        .unwrap();
        let p = plan(&conn, &kek, &bad, Mode::Merge).unwrap();
        let err = apply(&conn, &kek, &bad, &p, "test", 10).unwrap_err();
        assert!(err.contains("admin.allow_ips"), "{err}");
        let rows: i64 = conn
            .query_row("SELECT COUNT(*) FROM config", [], |r| r.get(0))
            .unwrap();
        assert_eq!(rows, 0, "nothing written");
    }

    #[test]
    fn a_wrong_kek_never_reads_as_unchanged() {
        let (_d, conn, kek) = store();
        let both = validate(&payload(
            serde_json::json!({"net.port": "18318", "net.admin_port": "18319"}),
        ))
        .unwrap();
        let p = plan(&conn, &kek, &both, Mode::Merge).unwrap();
        apply(&conn, &kek, &both, &p, "test", 10).unwrap();
        let items = validate(&payload(serde_json::json!({"net.port": "18318"}))).unwrap();
        let other = Kek::from_bytes([9u8; 32]);
        let p = plan(&conn, &other, &items, Mode::Merge).unwrap();
        assert_eq!(p, vec![("net.port".to_string(), Action::Set)]);
        // The untouched row cannot be opened with the wrong KEK, so the
        // boot-load check refuses the import and nothing is written.
        assert!(apply(&conn, &other, &items, &p, "test", 11).is_err());
        let p = plan(&conn, &kek, &both, Mode::Merge).unwrap();
        assert!(p.iter().all(|(_, a)| *a == Action::Unchanged), "{p:?}");
    }
}
