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

    let store = Arc::new(Mutex::new(store_mgr));
    let server = MemoryServer::new(store.clone());
    let transport = rmcp::transport::io::stdio();
    let service = rmcp::serve_server(server, transport).await.map_err(|e| {
        tracing::error!("MCP server failed to start: {e}");
        e
    })?;

    tokio::select! {
        result = service.waiting() => {
            if let Err(e) = result {
                tracing::error!("MCP server error: {e}");
            }
        }
        _ = shutdown_signal() => {
            tracing::info!("shutdown signal received");
        }
    }
    // service is dropped here — rmcp handles transport cleanup on drop

    // Explicit cleanup — recover from poisoned mutex (matches tools.rs pattern).
    // std::sync::Mutex::lock() is safe here on the async thread because:
    // 1. The MCP service is dropped (select! completed), so no new tool calls start.
    // 2. spawn_blocking tasks complete before the tokio runtime drops.
    // 3. Therefore no other thread holds the lock at this point.
    let mut mgr = match store.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("mutex poisoned, recovering for cleanup");
            poisoned.into_inner()
        }
    };
    if let Err(e) = mgr.close_active() {
        tracing::warn!(error = %e, "shutdown cleanup failed");
    }
    tracing::info!("local-memory-mcp stopped");

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => {
                sig.recv().await;
            }
            Err(e) => {
                tracing::warn!("failed to install SIGTERM handler: {e}");
                std::future::pending::<()>().await;
            }
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {}
        _ = terminate => {}
    }
}
