//! Read-only MCP stub (F12 slice): ONE tool, `lookup_authority`.
//!
//! The tool answers from **accepted facts only** — proposed,
//! community, rejected, and superseded rows are never exposed — and
//! every answer carries `citations` plus the publishing `build_no` it
//! was read from. Authorities with F6 conflicts carry a
//! `conflict_banner` naming the contradictory accepted statements
//! (banner policy, never silent).
//!
//! Transport is JSON-RPC 2.0 over HTTP (`tools/list`, `tools/call`),
//! served by [`serve`] on **localhost only**: the listener binds
//! `127.0.0.1`, and non-loopback peers are refused fail-closed.

use std::net::{IpAddr, SocketAddr};
use std::path::PathBuf;

use axum::{
    Router,
    extract::{ConnectInfo, State},
    http::StatusCode,
    response::{IntoResponse, Json},
    routing::post,
};
use handbook_store::store::Store;
use serde::{Deserialize, Serialize};

/// The single tool this stub exposes.
pub const TOOL_NAME: &str = "lookup_authority";

/// One accepted fact with its citation, as served to MCP clients.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CitedFact {
    pub topic: String,
    pub statement: String,
    pub capture_id: i64,
    pub capture_sha: String,
}

/// `lookup_authority` result: accepted facts, citations, build number,
/// and the conflict banner (null when the authority is consistent).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LookupResult {
    pub slug: String,
    pub name: String,
    pub build_no: i64,
    pub facts: Vec<CitedFact>,
    pub conflict_banner: Option<String>,
}

/// Read one authority's accepted facts for the tool.
///
/// `build_no` is the latest published build (0 when nothing has been
/// published yet: the answer is then explicitly unbuilt, never
/// misattributed). Unknown slugs are `None`.
///
/// # Errors
///
/// Returns [`contracts::HandbookError`] on database failures.
pub fn lookup_authority(
    conn: &rusqlite::Connection,
    slug: &str,
) -> contracts::Result<Option<LookupResult>> {
    let authority: Option<(i64, String)> = conn
        .query_row(
            "SELECT id, name FROM authority WHERE slug = ?1",
            [slug],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .map(Some)
        .or_else(|e| match e {
            rusqlite::Error::QueryReturnedNoRows => Ok(None),
            other => Err(other),
        })?;
    let Some((id, name)) = authority else {
        return Ok(None);
    };
    let facts = publish::accepted_facts(conn, id)?
        .into_iter()
        .map(|f| CitedFact {
            topic: f.topic,
            statement: f.statement,
            capture_id: f.capture_id,
            capture_sha: f.capture_sha,
        })
        .collect();
    let build_no: i64 =
        conn.query_row("SELECT COALESCE(MAX(build_no), 0) FROM build", [], |row| {
            row.get(0)
        })?;
    let mine: Vec<publish::Conflict> = publish::conflicts(conn)?
        .into_iter()
        .filter(|c| c.authority_id == id)
        .collect();
    let conflict_banner = (!mine.is_empty()).then(|| {
        let parts: Vec<String> = mine
            .iter()
            .map(|c| {
                format!(
                    "topic {}: #{} vs #{} (captures {} and {})",
                    c.topic, c.first_id, c.second_id, c.first_capture_id, c.second_capture_id
                )
            })
            .collect();
        format!("conflicting accepted records: {}", parts.join("; "))
    });
    Ok(Some(LookupResult {
        slug: slug.to_string(),
        name,
        build_no,
        facts,
        conflict_banner,
    }))
}

#[derive(Clone)]
struct AppState {
    store_root: PathBuf,
}

/// JSON-RPC 2.0 request (subset: `tools/list`, `tools/call`).
#[derive(Debug, Deserialize)]
struct RpcRequest {
    id: serde_json::Value,
    method: String,
    #[serde(default)]
    params: serde_json::Value,
}

/// Tool-call arguments for `lookup_authority`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct LookupArgs {
    slug: String,
}

fn rpc_result(id: &serde_json::Value, result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({"jsonrpc": "2.0", "id": id, "result": result})
}

fn rpc_error(
    id: &serde_json::Value,
    code: i64,
    message: &str,
) -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::OK, // JSON-RPC errors ride HTTP 200 by spec.
        Json(serde_json::json!({
            "jsonrpc": "2.0", "id": id,
            "error": {"code": code, "message": message},
        })),
    )
}

/// Build the MCP router (one POST route). `store_root` locates the
/// SQLite metadata DB; handlers open a connection per request.
pub fn router(store_root: PathBuf) -> Router {
    Router::new()
        .route("/mcp", post(dispatch))
        .with_state(AppState { store_root })
}

async fn dispatch(
    State(state): State<AppState>,
    ConnectInfo(peer): ConnectInfo<SocketAddr>,
    Json(req): Json<RpcRequest>,
) -> impl IntoResponse {
    // Localhost-only, defense in depth: the listener binds 127.0.0.1,
    // and any non-loopback peer that somehow arrives is refused.
    if !is_loopback(&peer.ip()) {
        return (
            StatusCode::FORBIDDEN,
            Json(serde_json::json!({"error": "localhost only"})),
        )
            .into_response();
    }
    match req.method.as_str() {
        "tools/list" => Json(rpc_result(
            &req.id,
            serde_json::json!({"tools": [{
                "name": TOOL_NAME,
                "description": "Look up one authority's accepted handbook facts with citations.",
                "inputSchema": {
                    "type": "object",
                    "properties": {"slug": {"type": "string"}},
                    "required": ["slug"],
                    "additionalProperties": false,
                },
            }]}),
        ))
        .into_response(),
        "tools/call" => call_tool(&state, &req).into_response(),
        _ => rpc_error(&req.id, -32601, "unknown method").into_response(),
    }
}

fn call_tool(state: &AppState, req: &RpcRequest) -> impl IntoResponse {
    let name = req.params.get("name").and_then(serde_json::Value::as_str);
    if name != Some(TOOL_NAME) {
        return rpc_error(&req.id, -32602, "unknown tool").into_response();
    }
    let args: LookupArgs =
        match serde_json::from_value(req.params.get("arguments").cloned().unwrap_or_default()) {
            Ok(args) => args,
            Err(_) => return rpc_error(&req.id, -32602, "invalid arguments").into_response(),
        };
    if args.slug.is_empty() || args.slug.len() > 128 {
        return rpc_error(&req.id, -32602, "invalid slug").into_response();
    }
    let store = Store::new(&state.store_root);
    let conn = match handbook_store::db::open(&store) {
        Ok(conn) => conn,
        Err(_) => return rpc_error(&req.id, -32000, "store unavailable").into_response(),
    };
    match lookup_authority(&conn, &args.slug) {
        Ok(Some(result)) => Json(rpc_result(
            &req.id,
            serde_json::to_value(result).unwrap_or_default(),
        ))
        .into_response(),
        Ok(None) => rpc_error(&req.id, -32004, "unknown authority").into_response(),
        Err(_) => rpc_error(&req.id, -32000, "store unavailable").into_response(),
    }
}

fn is_loopback(ip: &IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => v4.is_loopback(),
        IpAddr::V6(v6) => v6.is_loopback(),
    }
}

/// Serve the MCP stub on 127.0.0.1:`port` until Ctrl-C. `port` 0 picks
/// an ephemeral port (for tests); the bound address is printed to
/// stderr so callers can discover it.
pub async fn serve(store_root: PathBuf, port: u16) -> std::io::Result<()> {
    let listener =
        tokio::net::TcpListener::bind(SocketAddr::new(IpAddr::from([127, 0, 0, 1]), port)).await?;
    eprintln!(
        "mcp listening on http://{}/mcp (localhost only)",
        listener.local_addr()?
    );
    axum::serve(
        listener,
        router(store_root).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(async {
        tokio::signal::ctrl_c().await.ok();
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use handbook_store::db::{self, NewAuthority};

    fn seeded() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().join("store");
        let store = Store::new(&root);
        store.init().unwrap();
        let conn = db::open(&store).unwrap();
        db::migrate(&conn).unwrap();
        let wa = db::add_authority(
            &conn,
            &NewAuthority {
                kind: "state",
                country: "USA",
                region: "Washington",
                name: "WA Secretary of State",
                slug: "usa-wa-sos",
                actor: "test",
                at: 1,
            },
        )
        .unwrap();
        for (i, sha) in ["a".repeat(64), "b".repeat(64)].iter().enumerate() {
            conn.execute(
                "INSERT INTO capture(authority_id, captured_at, kind, path,
                                     bytes_raw, bytes_stored, sha256, trust)
                 VALUES (?1, 1, 'file', ?2, 1, 1, ?3, 'staff_verified')",
                rusqlite::params![wa, format!("blob/{i}"), sha],
            )
            .unwrap();
        }
        for (statement, status, capture) in [
            ("PO Box 40220", "accepted", 1),
            ("PO Box 40234", "accepted", 2),
        ] {
            conn.execute(
                "INSERT INTO fact(authority_id, capture_id, topic, statement,
                                  status, created_by, created_at)
                 VALUES (?1, ?2, 'po_box', ?3, ?4, 'test', 1)",
                rusqlite::params![wa, capture, statement, status],
            )
            .unwrap();
        }
        conn.execute(
            "INSERT INTO build(at, code_git_commit, build_no, facts_hash)
             VALUES (1, '', 7, 'hash')",
            [],
        )
        .unwrap();
        (dir, root)
    }

    /// Conformance shape over a live localhost listener (port 0):
    /// citations + build_no present, conflict banner set on WA.
    #[tokio::test]
    async fn live_localhost_roundtrip() {
        // NOTE: oneshot cannot drive `dispatch` (ConnectInfo needs a
        // real peer), so conformance runs over a live localhost port 0.
        let (_dir, root) = seeded();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(
                listener,
                router(root).into_make_service_with_connect_info::<SocketAddr>(),
            )
            .await
            .unwrap();
        });
        let client = reqwestless_post(
            addr,
            serde_json::json!({
                "jsonrpc": "2.0", "id": 1, "method": "tools/call",
                "params": {"name": "lookup_authority", "arguments": {"slug": "usa-wa-sos"}},
            }),
        )
        .await;
        assert_eq!(client["result"]["slug"], "usa-wa-sos");
        assert_eq!(client["result"]["build_no"], 7);
        assert_eq!(client["result"]["facts"].as_array().unwrap().len(), 2);
        for fact in client["result"]["facts"].as_array().unwrap() {
            assert!(fact.get("capture_id").is_some(), "{fact}");
            assert!(fact.get("capture_sha").is_some(), "{fact}");
        }
        assert!(
            client["result"]["conflict_banner"]
                .as_str()
                .unwrap()
                .contains("po_box"),
            "{client}"
        );
        // tools/list advertises exactly the one tool.
        let list = reqwestless_post(
            addr,
            serde_json::json!({
                "jsonrpc": "2.0", "id": 2, "method": "tools/list",
            }),
        )
        .await;
        assert_eq!(list["result"]["tools"].as_array().unwrap().len(), 1);
        assert_eq!(list["result"]["tools"][0]["name"], "lookup_authority");
        // Unknown tool and unknown authority are JSON-RPC errors.
        let bad_tool = reqwestless_post(
            addr,
            serde_json::json!({
                "jsonrpc": "2.0", "id": 3, "method": "tools/call",
                "params": {"name": "drop_tables", "arguments": {}},
            }),
        )
        .await;
        assert_eq!(bad_tool["error"]["code"], -32602);
        let bad_slug = reqwestless_post(
            addr,
            serde_json::json!({
                "jsonrpc": "2.0", "id": 4, "method": "tools/call",
                "params": {"name": "lookup_authority", "arguments": {"slug": "nope"}},
            }),
        )
        .await;
        assert_eq!(bad_slug["error"]["code"], -32004);
        server.abort();
    }

    /// Minimal HTTP/1.0 POST client over tokio (no client dependency).
    async fn reqwestless_post(addr: SocketAddr, body: serde_json::Value) -> serde_json::Value {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let rendered = body.to_string();
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        let req = format!(
            "POST /mcp HTTP/1.1\r\nhost: {addr}\r\ncontent-type: application/json\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{rendered}",
            rendered.len(),
        );
        stream.write_all(req.as_bytes()).await.unwrap();
        let mut raw = Vec::new();
        stream.read_to_end(&mut raw).await.unwrap();
        let text = String::from_utf8(raw).unwrap();
        let json_start = text.find('{').unwrap();
        serde_json::from_str(&text[json_start..]).unwrap()
    }

    #[test]
    fn lookup_rejects_non_loopback() {
        assert!(is_loopback(&"127.0.0.1".parse().unwrap()));
        assert!(is_loopback(&"::1".parse().unwrap()));
        assert!(!is_loopback(&"10.0.0.1".parse().unwrap()));
    }
}
