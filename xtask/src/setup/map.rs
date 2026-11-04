//! The wizard's system map: a UML deployment view of the whole system,
//! drawn by the page from [`state`].
//!
//! Three nodes (front end, workers, staging) each run on a target of
//! the setup file. A node is **Local** (a `local` target, natively or in
//! Docker: its `runtime`) or **Server** (a `server` target for the
//! front end, a `plant` target for staging and workers). The choice is
//! kept in the file's `map` section; unset, [`resolve`] derives it.
//! Around them: the Meta Model API the front end and workers call, the
//! Hugging Face weights the workers serve, and the outside services the
//! front end uses, each with its console and, when the file says, its
//! mode.

use std::collections::BTreeMap;

use serde_json::{Value, json};

use super::file::{Runtime, SetupFile, Target, TargetKind};

pub const NODES: [&str; 3] = ["front_end", "workers", "staging"];

/// Where the workers keep model weights (operator decision 2026-11-03,
/// `ops/open-weights/glimmer-serve.sh`).
pub const MODELS_DIR: &str = "/srv/scratch/apostille/models";

/// Meta model ids each node calls (the `«use»` labels). The web server
/// drafts, plans, translates and checks facts (`crates/web`) and
/// transcribes calls (`crates/media`); the workers' research and
/// discovery kinds draft with Spark (the atlas pack's models).
const FRONT_END_MODELS: &[&str] = &[
    handbook_agent::replay::REPLAY_MODEL,
    "muse-wonder-1.0",
    media::transcribe::VOICE_MODEL,
];
const WORKER_MODELS: &[&str] = &[handbook_agent::replay::REPLAY_MODEL];

/// Outside services on the map: id, name, console, the keys that
/// configure it.
const SERVICES: &[(&str, &str, &str, &[&str])] = &[
    (
        "gmail",
        "Gmail",
        "https://console.cloud.google.com/apis/credentials",
        &["mail.client_id", "mail.client_secret", "mail.refresh_token"],
    ),
    (
        "telnyx",
        "Telnyx",
        "https://portal.telnyx.com/",
        &[
            "telnyx.api_key",
            "telnyx.connection_id",
            "telnyx.from_number",
            "telnyx.webhook_public_key",
            "telnyx.webrtc_connection_id",
        ],
    ),
    (
        "fedex",
        "FedEx",
        "https://developer.fedex.com/",
        &[
            "labels.fedex_client_id",
            "labels.fedex_client_secret",
            "labels.fedex_account",
        ],
    ),
    (
        "usps",
        "USPS",
        "https://developers.usps.com/",
        &["usps.client_id", "usps.client_secret"],
    ),
    (
        "stripe",
        "Stripe",
        "https://dashboard.stripe.com/",
        &["stripe.secret_key", "stripe.webhook_secret"],
    ),
];

/// Kinds a node takes when it is placed on a server, in preference order.
fn server_kinds(node: &str) -> &'static [TargetKind] {
    match node {
        "front_end" => &[TargetKind::Server],
        _ => &[TargetKind::Plant],
    }
}

fn first(file: &SetupFile, kinds: &[TargetKind]) -> Option<String> {
    kinds.iter().find_map(|k| {
        file.targets
            .iter()
            .find(|(_, t)| t.kind == *k)
            .map(|(name, _)| name.clone())
    })
}

/// The target each node runs on: the file's choice, else the server
/// target when there is one, else the local target. Workers follow a
/// local front end (`xtask up` and the judge image run one) and run on
/// plant beside a server.
pub fn resolve(file: &SetupFile) -> BTreeMap<&'static str, Option<String>> {
    let known = |name: &Option<String>| name.clone().filter(|n| file.targets.contains_key(n));
    let front_end = known(&file.map.front_end)
        .or_else(|| first(file, &[TargetKind::Server, TargetKind::Local]))
        .or_else(|| file.targets.keys().next().cloned());
    let front_kind = front_end.as_ref().map(|n| file.targets[n].kind);
    let workers = known(&file.map.workers).or_else(|| match front_kind {
        Some(TargetKind::Server) => first(file, &[TargetKind::Plant]).or(front_end.clone()),
        _ => front_end.clone(),
    });
    let staging = known(&file.map.staging).or_else(|| first(file, &[TargetKind::Plant]));
    BTreeMap::from([
        ("front_end", front_end),
        ("workers", workers),
        ("staging", staging),
    ])
}

/// How the map shows a target: `native`, `docker` or `server`.
pub fn place(target: &Target) -> &'static str {
    match target.kind {
        TargetKind::Local if target.runtime == Runtime::Docker => "docker",
        TargetKind::Local => "native",
        TargetKind::Docker => "docker",
        TargetKind::Plant | TargetKind::Server => "server",
    }
}

/// Why a node cannot go on a server in this file, if it cannot.
fn no_server(file: &SetupFile, node: &str) -> Option<String> {
    if first(file, server_kinds(node)).is_some() {
        return None;
    }
    let kind = if node == "front_end" {
        "server"
    } else {
        "plant"
    };
    let path = super::file::DEFAULT_PATH;
    // Only local targets: the plain file, which can hold nothing else.
    Some(
        if file.targets.values().all(|t| t.kind == TargetKind::Local) {
            format!(
                "a {kind} target needs the sops file {path} (install sops and age, then `cargo xtask setup new`)"
            )
        } else {
            format!("this file has no {kind} target (add one with `sops {path}`)")
        },
    )
}

/// The operator's toggle: `native` or `docker` puts the node on the
/// local target and sets how that target runs; `server` puts it on the
/// first server (front end) or plant (staging, workers) target.
pub fn choose(file: &mut SetupFile, node: &str, place: &str) -> Result<(), String> {
    if !NODES.contains(&node) {
        return Err(format!("unknown map node {node:?}"));
    }
    let target = match place {
        "native" | "docker" => {
            let name = first(file, &[TargetKind::Local]).ok_or("this file has no local target")?;
            file.targets.get_mut(&name).expect("just found").runtime = if place == "docker" {
                Runtime::Docker
            } else {
                Runtime::Native
            };
            name
        }
        "server" => first(file, server_kinds(node))
            .ok_or_else(|| no_server(file, node).unwrap_or_default())?,
        _ => {
            return Err(format!(
                "unknown place {place:?} (native, docker or server)"
            ));
        }
    };
    let slot = match node {
        "front_end" => &mut file.map.front_end,
        "workers" => &mut file.map.workers,
        _ => &mut file.map.staging,
    };
    *slot = Some(target);
    Ok(())
}

/// `org/name` on the Hugging Face hub.
pub fn valid_repo(repo: &str) -> bool {
    let mut parts = repo.split('/');
    let ok = |p: Option<&str>| {
        p.is_some_and(|p| {
            !p.is_empty()
                && p.len() <= 96
                && !p.starts_with(['.', '-'])
                && p.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
        })
    };
    ok(parts.next()) && ok(parts.next()) && parts.next().is_none()
}

/// Public site and admin console of a target, as a browser opens them.
fn urls(target: &Target) -> (String, String) {
    match target.kind {
        TargetKind::Local if target.runtime == Runtime::Docker => (
            "http://localhost:18318/apostille".into(),
            "http://localhost:18319".into(),
        ),
        TargetKind::Local | TargetKind::Plant => {
            let p = target.ports_or_default();
            (
                format!("http://localhost:{}/apostille", p.public),
                format!("http://localhost:{}", p.admin),
            )
        }
        TargetKind::Docker => {
            let host = target
                .public_host
                .clone()
                .unwrap_or_else(|| "rehearsal.localhost".into());
            let port = target.publish.unwrap_or_default().https;
            let port = if port == 443 {
                String::new()
            } else {
                format!(":{port}")
            };
            (
                format!("https://{host}{port}/apostille"),
                format!("https://admin.{host}{port}"),
            )
        }
        TargetKind::Server => {
            let host = target.public_host.clone().unwrap_or_default();
            (
                format!("https://{host}/apostille"),
                format!("https://admin.{host}"),
            )
        }
    }
}

fn link(label: &str, href: String) -> Value {
    json!({ "label": label, "href": href })
}

/// One node as the page draws it.
fn node(file: &SetupFile, node: &str, name: Option<&String>, front_admin: &str) -> Value {
    let can = json!({
        "native": first(file, &[TargetKind::Local]).is_some(),
        "docker": first(file, &[TargetKind::Local]).is_some(),
        "server": no_server(file, node).is_none(),
    });
    let Some(name) = name else {
        return json!({ "target": null, "can": can, "server_hint": no_server(file, node) });
    };
    let t = &file.targets[name];
    let (public, admin) = urls(t);
    let host = match t.kind {
        TargetKind::Server | TargetKind::Docker => t.public_host.clone(),
        TargetKind::Plant => t.ssh.clone(),
        TargetKind::Local => Some("localhost".into()),
    };
    let (links, artifacts, note) = match node {
        "front_end" => (
            vec![
                link("Open the public site", public),
                link("Open the admin console", admin),
            ],
            "web · public + admin sites",
            None,
        ),
        "workers" => (
            vec![link("Work queue", format!("{front_admin}/work"))],
            "xtask worker · model weights",
            Some(match t.kind {
                TargetKind::Local if t.runtime == Runtime::Docker => {
                    "the judge image runs one".to_string()
                }
                TargetKind::Local => "xtask up runs one".to_string(),
                _ => format!("xtask worker on {}", t.ssh.as_deref().unwrap_or(name)),
            }),
        ),
        _ => (
            vec![link("Staging", public)],
            "web (staging build)",
            (t.kind == TargetKind::Plant).then(|| {
                let port = t.ports_or_default().public;
                format!(
                    "ssh -L {port}:127.0.0.1:{port} {}",
                    t.ssh.as_deref().unwrap_or("plant")
                )
            }),
        ),
    };
    json!({
        "target": name,
        "kind": t.kind,
        "place": place(t),
        "host": host,
        "ssh": t.ssh,
        "probe": t.probe,
        "links": links,
        "artifacts": artifacts,
        "note": note,
        "can": can,
        "server_hint": no_server(file, node),
    })
}

/// The mode a service runs in, when the file says: its W-3 `<area>.mode`
/// row (`fake | sandbox | live`) first, else what its keys imply.
fn mode(id: &str, values: &BTreeMap<String, Value>) -> Option<&'static str> {
    let text = |k: &str| values.get(k).and_then(Value::as_str).unwrap_or_default();
    let live = |k: &str| values.get(k).and_then(Value::as_bool);
    let rows: &[&str] = match id {
        "gmail" => &["mail.mode", "mailer.mode"],
        "telnyx" => &["telnyx.mode"],
        "fedex" | "usps" => &["labels.mode"],
        "stripe" => &["stripe.mode"],
        _ => &[],
    };
    let set = rows.iter().find_map(|row| {
        let value = text(row).trim();
        ["fake", "sandbox", "live"]
            .into_iter()
            .find(|m| *m == value)
    });
    if set.is_some() {
        return set;
    }
    match id {
        "stripe" => {
            let key = text("stripe.secret_key");
            if key.starts_with("sk_live_") || key.starts_with("rk_live_") {
                Some("live")
            } else if key.starts_with("sk_test_") || key.starts_with("rk_test_") {
                Some("sandbox")
            } else {
                None
            }
        }
        "fedex" => live("labels.live")
            .map(|on| if on { "live" } else { "sandbox" })
            .or_else(|| {
                let base = text("labels.fedex_base_url");
                (!base.is_empty()).then(|| {
                    if base.contains("sandbox") {
                        "sandbox"
                    } else {
                        "live"
                    }
                })
            }),
        "usps" => live("usps.live").map(|on| if on { "live" } else { "sandbox" }),
        _ => None,
    }
}

/// Everything the page needs to draw the map. Secrets never leave:
/// services say only whether they are configured and their mode.
pub fn state(file: &SetupFile) -> Value {
    let placed = resolve(file);
    let front = placed["front_end"].as_ref();
    let front_admin = front.map(|n| urls(&file.targets[n]).1).unwrap_or_default();
    let nodes: BTreeMap<&str, Value> = NODES
        .iter()
        .map(|n| (*n, node(file, n, placed[n].as_ref(), &front_admin)))
        .collect();
    let values = front
        .and_then(|n| file.target_config(n).ok())
        .unwrap_or_default();
    let set = |k: &str| {
        values
            .get(k)
            .is_some_and(|v| !matches!(v, Value::Null) && v.as_str() != Some(""))
    };
    let services: Vec<Value> = SERVICES
        .iter()
        .map(|(id, name, console, keys)| {
            json!({
                "id": id, "name": name, "console": console,
                "configured": keys.iter().any(|k| set(k)),
                "mode": mode(id, &values),
            })
        })
        .collect();
    let worker_host = placed["workers"]
        .as_ref()
        .map(|n| {
            let t = &file.targets[n];
            match t.kind {
                TargetKind::Local => "localhost".to_string(),
                _ => t.ssh.clone().unwrap_or_else(|| n.clone()),
            }
        })
        .unwrap_or_else(|| "worker".into());
    json!({
        "nodes": nodes,
        "models": { "front_end": FRONT_END_MODELS, "workers": WORKER_MODELS },
        "meta": {
            "target": front,
            "base_url": values.get("meta.base_url").and_then(Value::as_str)
                .unwrap_or(handbook_agent::model::META_BASE_URL),
            "key": set("meta.api_key"),
        },
        "weights": {
            "repo": file.map.weights_repo,
            "dest": format!("{worker_host}:{MODELS_DIR}"),
        },
        "services": services,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nodes_resolve_to_the_server_beside_plant_or_all_local() {
        let file = SetupFile::template();
        let placed = resolve(&file);
        assert_eq!(placed["front_end"].as_deref(), Some("production"));
        assert_eq!(placed["workers"].as_deref(), Some("staging"));
        assert_eq!(placed["staging"].as_deref(), Some("staging"));
        let local = SetupFile::local_template();
        let placed = resolve(&local);
        assert_eq!(placed["front_end"].as_deref(), Some("local"));
        assert_eq!(placed["workers"].as_deref(), Some("local"));
        assert_eq!(placed["staging"], None);
    }

    #[test]
    fn toggles_pick_targets_and_the_local_runtime() {
        let mut file = SetupFile::template();
        choose(&mut file, "front_end", "docker").unwrap();
        assert_eq!(file.map.front_end.as_deref(), Some("local"));
        assert_eq!(file.targets["local"].runtime, Runtime::Docker);
        assert_eq!(place(&file.targets["local"]), "docker");
        file.validate().unwrap();
        let yaml = serde_yaml_ng::to_string(&file).unwrap();
        assert!(
            yaml.contains("runtime: docker") && yaml.contains("front_end: local"),
            "{yaml}"
        );
        choose(&mut file, "front_end", "server").unwrap();
        assert_eq!(file.map.front_end.as_deref(), Some("production"));
        choose(&mut file, "workers", "native").unwrap();
        assert_eq!(file.targets["local"].runtime, Runtime::Native);
        assert!(choose(&mut file, "staging", "cloud").is_err());
        assert!(choose(&mut file, "database", "server").is_err());

        let mut plain = SetupFile::local_template();
        let err = choose(&mut plain, "front_end", "server").unwrap_err();
        assert!(err.contains("sops"), "{err}");
        let mut no_host = SetupFile::template_for(None);
        let err = choose(&mut no_host, "front_end", "server").unwrap_err();
        assert!(err.contains("no server target"), "{err}");

        let mut bad = SetupFile::template();
        bad.map.front_end = Some("gone".into());
        bad.targets.get_mut("staging").unwrap().runtime = Runtime::Docker;
        let err = bad.validate().unwrap_err().join("; ");
        assert!(
            err.contains("map.front_end") && err.contains("runtime"),
            "{err}"
        );
    }

    #[test]
    fn the_map_shows_hosts_links_models_and_modes_but_no_secret() {
        let mut file = SetupFile::template();
        let prod = file.targets.get_mut("production").unwrap();
        prod.config.insert(
            "stripe.secret_key".into(),
            Value::String(format!("sk_{}_synthetic_never_shown", "test")),
        );
        prod.config.insert("usps.live".into(), Value::Bool(true));
        prod.config.insert(
            "meta.api_key".into(),
            Value::String("LLM_1_synthetic_never_shown".into()),
        );
        file.map.weights_repo = Some("synthetic-org/glimmer-gguf".into());
        let s = state(&file);
        let text = s.to_string();
        assert!(
            !text.contains("never_shown"),
            "a secret reached the map: {text}"
        );
        let fe = &s["nodes"]["front_end"];
        assert_eq!(fe["place"], "server");
        assert_eq!(fe["host"], "andrii.ai");
        assert_eq!(fe["links"][0]["href"], "https://andrii.ai/apostille");
        assert_eq!(fe["links"][1]["href"], "https://admin.andrii.ai");
        assert_eq!(
            s["nodes"]["workers"]["links"][0]["href"],
            "https://admin.andrii.ai/work"
        );
        assert_eq!(
            s["nodes"]["staging"]["links"][0]["href"],
            "http://localhost:18318/apostille"
        );
        assert_eq!(
            s["nodes"]["staging"]["note"],
            "ssh -L 18318:127.0.0.1:18318 plant"
        );
        assert_eq!(s["weights"]["dest"], "plant:/srv/scratch/apostille/models");
        assert_eq!(s["meta"]["key"], true);
        assert!(
            s["models"]["front_end"]
                .as_array()
                .unwrap()
                .iter()
                .all(|m| m.as_str().unwrap().starts_with("muse-"))
        );
        let mode = |id: &str| {
            s["services"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["id"] == id)
                .unwrap()
                .clone()
        };
        assert_eq!(mode("stripe")["mode"], "sandbox");
        assert_eq!(mode("stripe")["configured"], true);
        assert_eq!(mode("usps")["mode"], "live");
        assert_eq!(mode("gmail")["configured"], false);
        assert_eq!(mode("gmail")["mode"], Value::Null);

        // A W-3 mode row wins over what the keys imply.
        let mut faked = file.clone();
        let prod = faked.targets.get_mut("production").unwrap();
        for (row, value) in [
            ("stripe.mode", "fake"),
            ("labels.mode", "fake"),
            ("mailer.mode", "fake"),
        ] {
            prod.config.insert(row.into(), Value::String(value.into()));
        }
        prod.config
            .insert("telnyx.mode".into(), Value::String("bogus".into()));
        let f = state(&faked);
        let fmode = |id: &str| {
            f["services"]
                .as_array()
                .unwrap()
                .iter()
                .find(|x| x["id"] == id)
                .unwrap()["mode"]
                .clone()
        };
        for id in ["stripe", "fedex", "usps", "gmail"] {
            assert_eq!(fmode(id), "fake", "{id}");
        }
        assert_eq!(fmode("telnyx"), Value::Null, "an unknown mode is not shown");

        let mut local = SetupFile::local_template();
        choose(&mut local, "front_end", "docker").unwrap();
        let s = state(&local);
        assert_eq!(
            s["nodes"]["front_end"]["links"][0]["href"],
            "http://localhost:18318/apostille"
        );
        assert_eq!(
            s["nodes"]["workers"]["links"][0]["href"],
            "http://localhost:18319/work"
        );
        assert_eq!(s["nodes"]["staging"]["target"], Value::Null);
        assert_eq!(s["nodes"]["staging"]["can"]["server"], false);
        assert!(
            s["nodes"]["staging"]["server_hint"]
                .as_str()
                .unwrap()
                .contains("sops")
        );
    }

    #[test]
    fn weights_repos_are_org_slash_name() {
        assert!(valid_repo("facebook/sam3"));
        assert!(valid_repo("synthetic-org/Glimmer_1.0-GGUF"));
        for bad in ["", "sam3", "a/b/c", "../x", "a/", "a b/c", "-a/b", "a/.b"] {
            assert!(!valid_repo(bad), "{bad}");
        }
    }

    /// The two shapes `web/e2e/xtask-wizard.spec.mjs` mocks: the
    /// operator's file (production beside plant, plant probed) and a
    /// judge's plain local file in Docker.
    fn fixture_files() -> (SetupFile, SetupFile) {
        let mut operator = SetupFile::template();
        let staging = operator.targets.get_mut("staging").unwrap();
        staging.probe = Some(super::super::probe::parse(
            "plant",
            "2026-11-04T09:00:00Z",
            "hostname=plant\nuser=andrii\nos=Ubuntu 24.04.1 LTS\nkernel=Linux 6.8.0-45-generic x86_64\n\
             cpus=16\nmem_kb=65779884\ndisk_free_kb=912345678\ngpu=GPU 0: NVIDIA GeForce RTX 3060\n\
             docker=Docker version 27.3.1, build ce12230\n",
        ));
        let prod = operator.targets.get_mut("production").unwrap();
        prod.config.insert(
            "stripe.secret_key".into(),
            Value::String(format!("sk_{}_synthetic", "test")),
        );
        prod.config.insert("usps.live".into(), Value::Bool(true));
        prod.config.insert(
            "meta.api_key".into(),
            Value::String("LLM_1_synthetic".into()),
        );
        let mut judge = SetupFile::local_template();
        choose(&mut judge, "front_end", "docker").unwrap();
        (operator, judge)
    }

    #[test]
    fn the_e2e_fixture_is_what_the_wizard_serves() {
        let (operator, judge) = fixture_files();
        let want = json!({ "operator": state(&operator), "judge": state(&judge) });
        let have: Value =
            serde_json::from_str(include_str!("../../../web/e2e/fixtures/wizard-map.json"))
                .unwrap();
        assert_eq!(
            have,
            want,
            "update web/e2e/fixtures/wizard-map.json to:\n{}",
            serde_json::to_string_pretty(&want).unwrap()
        );
    }
}
