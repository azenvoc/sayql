//! `sayql-mcp`: SayQL's MCP server for Claude, over stdio. See src/mcp/.

#[tokio::main]
async fn main() {
    if let Err(e) = sayql_lib::mcp::serve_stdio().await {
        // stdout carries the protocol; errors go to stderr.
        eprintln!("sayql-mcp: {e}");
        std::process::exit(1);
    }
}
