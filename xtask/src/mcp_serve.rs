//! MCP stub server CLI: `xtask mcp-serve [--store <path>] [--port <u16>]`.
//!
//! Serves the one read-only tool `lookup_authority` on 127.0.0.1 only
//! (enforced in the `mcp` crate: loopback bind + peer check).

/// CLI entry: serve until Ctrl-C.
pub fn run(args: &[String]) {
    let store = crate::open_store(args);
    let port: u16 = crate::flag(args, "port")
        .and_then(|p| p.parse().ok())
        .unwrap_or(3099);
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap_or_else(|e| {
            eprintln!("mcp-serve: runtime: {e}");
            std::process::exit(1);
        });
    if let Err(e) = runtime.block_on(mcp::serve(store.root().to_path_buf(), port)) {
        eprintln!("mcp-serve: {e}");
        std::process::exit(1);
    }
}
