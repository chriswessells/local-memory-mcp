# Graceful Shutdown — Design

## Problem

When the MCP server process is terminated by a signal (SIGTERM/SIGINT), no cleanup runs. The `StoreManager::Drop` implementation never executes because the OS kills the process before Rust can unwind the stack. This leaves the `-wal` file on disk without a final checkpoint.

### What actually happens on unclean shutdown

1. The OS kernel releases all POSIX advisory locks — SQLite's EXCLUSIVE lock is built on top of these, so **the database is NOT locked after process death**. A new process can always open it.
2. SQLite's WAL recovery automatically replays committed transactions from the leftover `-wal` file on next open. **No data loss for committed transactions.**
3. The `-shm` file is never created because we set `PRAGMA locking_mode = EXCLUSIVE` before the first WAL access (per SQLite docs §8).

### So what's the actual problem?

The database is recoverable, but without a clean shutdown:

- **No WAL checkpoint** — the `-wal` file grows unbounded across restarts, accumulating uncheckpointed pages.
- **No `PRAGMA optimize`** — SQLite never gets a chance to update query planner statistics.
- **No tracing** — the server disappears silently with no shutdown log entry.

## Root Causes

1. **No signal handler in `main.rs`** — SIGTERM/SIGINT use the OS default action (immediate termination). No Rust code runs.
2. **No explicit cleanup after `service.waiting().await`** — even on normal MCP transport close, `main()` returns `Ok(())` without calling `close_active()`.
3. **`StoreManager` is inside `Arc<Mutex<>>`** — `Drop` only runs when the last `Arc` clone is dropped. With the `Arc` cloned into `MemoryServer` and potentially into spawned tasks, drop ordering is not guaranteed on abrupt exit.

## Solution

Add a signal handler using `tokio::signal` and `tokio::select!` so that SIGTERM/SIGINT cause `main()` to proceed to explicit cleanup instead of killing the process.

### Dependency change

Add `signal` feature to the existing tokio dependency in `Cargo.toml` (no new crate):

```toml
tokio = { version = "1", features = ["macros", "rt-multi-thread", "sync", "signal"] }
```

### Changes to `main.rs`

`main()` must retain its own `Arc` clone so it can access the store after the MCP service is dropped. Previously, the sole `Arc` was moved into `MemoryServer`.

```rust
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

    // Explicit cleanup — recover from poisoned mutex (matches tools.rs pattern)
    // spawn_blocking tasks complete before tokio runtime drops,
    // so the lock is guaranteed to be available here.
    let mut mgr = match store.lock() {
        Ok(guard) => guard,
        Err(poisoned) => {
            tracing::warn!("mutex poisoned, recovering for cleanup");
            poisoned.into_inner()
        }
    };
    if let Err(e) = mgr.close_active() {
        tracing::warn!("shutdown cleanup failed");
    }
    tracing::info!("local-memory-mcp stopped");

    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = tokio::signal::ctrl_c();

    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut sig) => { sig.recv().await; }
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
```

### What this changes

| Before | After |
|--------|-------|
| SIGTERM/SIGINT → immediate process death | SIGTERM/SIGINT → `shutdown_signal()` future completes, `select!` exits, cleanup runs |
| Normal MCP close → `main()` returns with no cleanup | Normal MCP close → `service.waiting()` completes, cleanup runs |
| `StoreManager::Drop` relied upon but never reached | Explicit `close_active()` call before exit |
| Sole `Arc` moved into `MemoryServer` | `main()` retains its own `Arc` clone for cleanup access |

### What `close_active()` does

1. `PRAGMA wal_checkpoint(TRUNCATE)` — flushes WAL to main DB, truncates `-wal` file to zero bytes
2. `PRAGMA optimize` — updates query planner statistics
3. Sets `active_store = None` — drops the `rusqlite::Connection`, which calls `sqlite3_close()` and releases all file locks

### Edge cases

- **SIGKILL**: Cannot be caught. The OS kills the process, releases POSIX locks, leaves `-wal` on disk. SQLite auto-recovers on next open. This is unavoidable and acceptable.
- **Poisoned mutex**: Recovered via `into_inner()`, matching the existing pattern in `tools.rs`. Safe because no other thread is using the store after the `select!` completes.
- **SIGTERM handler install failure**: Falls back to `std::future::pending()` with a warning log. Server still runs with SIGINT (Ctrl+C) handling only. This can happen in restricted container environments.
- **`close_active()` failure**: Leaves the connection open for `StoreManager::Drop` to retry — intentional defense-in-depth.
- **MCP service drop**: `service` is dropped at end of the `select!` scope. rmcp handles transport cleanup on drop. The MCP client sees EOF on stdio.
- **Cleanup timeout**: `close_active()` is synchronous and fast (WAL checkpoint + optimize). No timeout needed. If the disk is unresponsive, the process will hang — but that's a system-level problem, not something we should mask.

### What this does NOT change

- `StoreManager::Drop` remains as a best-effort fallback.
- `db::open()` still sets `PRAGMA locking_mode = EXCLUSIVE` and `BEGIN IMMEDIATE; COMMIT`.
- No PID file or application-level lock file — POSIX advisory locks + SQLite's built-in recovery are sufficient.
- No changes to `store.rs` or `db.rs`.

## Implementation Plan

1. Add `"signal"` feature to tokio dependency in `Cargo.toml`
2. Add `shutdown_signal()` async function to `main.rs`
3. Clone the `Arc<Mutex<StoreManager>>` before passing to `MemoryServer`
4. Replace `service.waiting().await?` with `tokio::select!` over `service.waiting()` and `shutdown_signal()`
5. Add explicit `close_active()` call after the `select!` with poisoned-mutex recovery
6. Add shutdown log lines
7. Verify: `cargo check`, `cargo test`, `cargo clippy -- -D warnings`

## Test Plan

- **Manual**: Run the server, send SIGTERM, verify `-wal` file is cleaned up and "stopped" log line appears on stderr.
- **Existing tests**: No test changes needed — this only modifies `main.rs` startup/shutdown flow. All existing unit and integration tests are unaffected.

## Medium/Low findings deferred to backlog

- No WAL checkpoint on startup after unclean shutdown (add `PRAGMA wal_checkpoint(TRUNCATE)` in `db::open()`)
- No automated e2e test for shutdown path (add SIGTERM test to `tests/e2e.rs`)
- No second-signal force-quit
- Windows `#[cfg(not(unix))]` branch is untested dead code
- No MCP-level graceful close notification before transport drop
