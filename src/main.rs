use local_memory_mcp::store::StoreManager;
use local_memory_mcp::tools::MemoryServer;
use std::sync::{Arc, Mutex};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    let mut store_mgr = StoreManager::new().map_err(|e| {
        eprintln!("Error: failed to initialize store: {e}");
        e
    })?;
    store_mgr.open_default().map_err(|e| {
        eprintln!("Error: failed to open default store: {e}");
        eprintln!("If another instance is running, stop it first.");
        e
    })?;

    tracing::info!(store = %store_mgr.active_name().unwrap_or("none"), "local-memory-mcp started");

    let server = MemoryServer::new(Arc::new(Mutex::new(store_mgr)));
    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport).await.map_err(|e| {
        tracing::error!("MCP server failed to start: {e}");
        e
    })?;
    service.waiting().await.map_err(|e| {
        tracing::error!("MCP server error: {e}");
        e
    })?;

    Ok(())
}
