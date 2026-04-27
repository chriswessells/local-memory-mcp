# Component 9: MCP Server — Detailed Design

## Review Resolution Log

### Critical

| # | Finding | Resolution |
|---|---------|------------|
| C1 | blob_data as `Vec<u8>` breaks JSON transport — JSON has no binary type, schemars generates array-of-integers schema (Interop #1) | Changed `blob_data` to `Option<String>` (base64-encoded) in MCP param struct. Decode in handler, encode in response. Added `#[schemars(description)]` annotation. |

### High

| # | Finding | Resolution |
|---|---------|------------|
| H1 | NaN/infinity embeddings not validated on store/consolidate path (Security #1) | Add `is_finite()` check to `validate_insert_memory_params` and `validate_consolidate_params` in `memories.rs`. Defense-in-depth alongside existing check in `search.rs`. |
| H2 | blob_data JSON encoding convention undefined (Security #2) | Resolved by C1 — base64 encoding. |
| H3 | `serde_json::to_string().unwrap()` in `run` helper can panic (Arch #4) | Replaced with `.map_err()` — serialization failures flow through normal error path. |
| H4 | `run` helper erases structured error context — client can't distinguish NotFound from InvalidInput (Arch #1) | Return JSON error body with `code` field: `{"code": "not_found", "message": "..."}`. Map MemoryError variants to machine-readable codes. |
| H5 | No test strategy for MCP tool layer (Maint #4) | Added tests for `run` helper (ok/error/panic paths), `parse_branch_filter`, `parse_consolidate_action`, and one end-to-end tool handler test. |
| H6 | No CI workflow (Maint #9) | Deferred to Component 10 (CI/CD) per TODO.md. Not a blocker for this component. |
| H7 | `process::exit(1)` on startup bypasses Drop (Reliability #1) | Changed main to return `Result` — normal drop semantics handle cleanup. |
| H8 | Locked DB produces opaque startup crash (Reliability #2) | Added human-readable error message to stderr before exit. |
| H9 | Embedding schema has no dimension constraint (Interop #2) | Added `#[schemars(extend("minItems" = 384, "maxItems" = 384))]` and description to all embedding fields. |
| H10 | String-typed enums lack JSON Schema enum constraints (Interop #4) | Changed `event_type`, `role`, `action` to Rust enums with `#[derive(Deserialize, JsonSchema)]`. `branch_filter` stays as String with description (has catch-all variant). |
| H11 | Tool descriptions too terse for LLM consumption (Interop #3) | Expanded all tool descriptions to 2-4 sentences with parameter constraints and return shape. |

### Medium (logged to TODO backlog)

- Error messages echo user-supplied IDs in NotFound (Security #3)
- ConnectionFailed/InvalidPath errors may leak filesystem paths (Security #4)
- Mutex poison recovery may hide panics — add `tracing::warn!` on recovery (Security #5, Arch #7, Reliability #4)
- Duplicated validation helpers across modules — extract to shared module (Maint #1)
- schemars version coupling with rmcp — pin as direct dep (Maint #5)
- Param struct proliferation — split tools.rs when >500 lines (Maint #3)
- Design doc self-contradictions — cleaned up in this revision (Maint #10)
- Shutdown drop ordering — add explicit drop after service.waiting() (Arch #6)
- `serve_server().unwrap()` — replace with proper error handling (Arch #11)
- `metadata` as String vs serde_json::Value for LLM usability (Interop #7)
- AgentCore Memory parameter naming divergences — document in descriptions (Interop #5)
- No MCP protocol-level error codes for tool failures (Interop #6)
- `valid_only` default not reflected in schema (Interop #13)
- `actor_id` required on consolidate/get/delete diverges from AgentCore (Interop #10)
- Tracing default level too quiet — change to `info` (Reliability #7)
- spawn_blocking panic recovery lacks logging (Reliability #3)
- WAL checkpoint depends on Drop running (Reliability #5)
- No startup readiness log message (Reliability #9)
- Auxiliary file deletion ignores errors (Reliability #10, Security #9)
- `LOCAL_MEMORY_SYNC` downgrade not warned (Security #8)
- Platform-specific base dir doesn't follow XDG/Apple conventions (Interop #8)

---

## Scope

Wire the existing business logic (events, memories, search, store management) into an MCP server over stdio. This component modifies:

1. **`src/main.rs`** — tokio runtime, tracing init, StoreManager init, serve MCP over stdio
2. **`src/tools.rs`** (new) — MCP tool definitions using `#[tool_router(server_handler)]` macro

This component does NOT implement:
- Knowledge graph tools (Component 5)
- Session tools / checkpoints / branches (Component 6)
- Namespace CRUD tools (Component 8)
- Stats / export / import tools (future)

Those will be added to `tools.rs` as their components are completed.

---

## Architecture

```
main.rs
  ├── init tracing (stderr)
  ├── StoreManager::new() + open_default()
  ├── wrap in Arc<std::sync::Mutex<StoreManager>>
  └── serve_server(MemoryServer, stdio())
          │
          ▼
tools.rs (MemoryServer struct)
  ├── #[tool_router(server_handler)]
  ├── Each #[tool] method:
  │     1. Deserialize params via Parameters<T>
  │     2. spawn_blocking { lock mutex, get db(), call business logic }
  │     3. Serialize result as JSON text content
  └── Error handling: MemoryError → CallToolResult with is_error=true
```

### Concurrency Model (from core-db-layer.md §2.1)

```rust
// MemoryServer holds:
store: Arc<std::sync::Mutex<StoreManager>>

// Each tool handler:
let result = tokio::task::spawn_blocking({
    let store = self.store.clone();
    move || {
        let mut mgr = store.lock().unwrap_or_else(|e| e.into_inner());
        let db = mgr.db()?;
        // call business logic
    }
}).await.map_err(|e| /* JoinError → internal error */)?;
```

All tool calls serialize through the mutex. Sub-millisecond SQLite ops make this acceptable for a single-user server.

---

## MemoryServer Struct

```rust
#[derive(Clone)]
pub struct MemoryServer {
    store: Arc<std::sync::Mutex<StoreManager>>,
}
```

`Clone` is required because `#[tool_router(server_handler)]` needs `ServerHandler: Send + Sync + 'static`. `Arc<Mutex<_>>` is `Clone + Send + Sync`.

---

## Tool Parameter Types

Each tool gets a dedicated `#[derive(Deserialize, schemars::JsonSchema)]` struct. These are MCP-facing DTOs — they map to/from JSON and convert to the internal business logic param types.

### Enums (H10 — proper JSON Schema enum constraints)

```rust
#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum EventType { Conversation, Blob }

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum Role { User, Assistant, Tool, System }

#[derive(Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
enum ConsolidateActionType { Update, Invalidate }
```

### Events

```rust
#[derive(Deserialize, JsonSchema)]
struct AddEventParams {
    actor_id: String,
    session_id: String,
    event_type: EventType,
    #[serde(default)]
    role: Option<Role>,
    #[serde(default)]
    content: Option<String>,
    /// Base64-encoded binary data for blob events
    #[serde(default)]
    blob_data: Option<String>,
    #[serde(default)]
    metadata: Option<String>,
    #[serde(default)]
    branch_id: Option<String>,
    #[serde(default)]
    expires_at: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct GetEventParams {
    actor_id: String,
    event_id: String,
}

#[derive(Deserialize, JsonSchema)]
struct GetEventsToolParams {
    actor_id: String,
    session_id: String,
    /// "all" (default), "main" (main timeline only), or a specific branch ID
    #[serde(default)]
    branch_filter: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
    #[serde(default)]
    before: Option<String>,
    #[serde(default)]
    after: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
struct ListSessionsParams {
    actor_id: String,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}
```

### Memories

```rust
#[derive(Deserialize, JsonSchema)]
struct StoreMemoryParams {
    actor_id: String,
    content: String,
    strategy: String,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    metadata: Option<String>,
    #[serde(default)]
    source_session_id: Option<String>,
    /// 384-dimensional float32 embedding vector
    #[schemars(extend("minItems" = 384, "maxItems" = 384))]
    #[serde(default)]
    embedding: Option<Vec<f32>>,
}

#[derive(Deserialize, JsonSchema)]
struct GetMemoryParams {
    actor_id: String,
    memory_id: String,
}

#[derive(Deserialize, JsonSchema)]
struct ListMemoriesToolParams {
    actor_id: String,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    namespace_prefix: Option<String>,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default = "default_true")]
    valid_only: bool,
    #[serde(default)]
    limit: Option<u32>,
    #[serde(default)]
    offset: Option<u32>,
}

#[derive(Deserialize, JsonSchema)]
struct ConsolidateParams {
    actor_id: String,
    memory_id: String,
    action: ConsolidateActionType,
    /// Required when action is "update"
    #[serde(default)]
    new_content: Option<String>,
    /// 384-dimensional float32 embedding vector for the replacement memory
    #[schemars(extend("minItems" = 384, "maxItems" = 384))]
    #[serde(default)]
    new_embedding: Option<Vec<f32>>,
}

#[derive(Deserialize, JsonSchema)]
struct DeleteMemoryParams {
    actor_id: String,
    memory_id: String,
}
```

### Search

```rust
#[derive(Deserialize, JsonSchema)]
struct RecallToolParams {
    actor_id: String,
    #[serde(default)]
    query: Option<String>,
    /// 384-dimensional float32 embedding vector
    #[schemars(extend("minItems" = 384, "maxItems" = 384))]
    #[serde(default)]
    embedding: Option<Vec<f32>>,
    #[serde(default)]
    namespace: Option<String>,
    #[serde(default)]
    namespace_prefix: Option<String>,
    #[serde(default)]
    strategy: Option<String>,
    #[serde(default)]
    limit: Option<u32>,
}
```

### Store Management

```rust
#[derive(Deserialize, JsonSchema)]
struct SwitchStoreParams {
    name: String,
}

#[derive(Deserialize, JsonSchema)]
struct DeleteStoreParams {
    name: String,
}
```

---

## Tool Definitions

### Tool naming

All tools use dotted names per DESIGN.md: `memory.add_event`, `memory.recall`, etc. rmcp validates tool names allow dots (confirmed in `tool_name_validation.rs`).

### Tool list (this component)

| Tool Name | Handler | Business Logic |
|-----------|---------|----------------|
| `memory.add_event` | `add_event` | `events::add_event` |
| `memory.get_event` | `get_event` | `events::get_event` |
| `memory.get_events` | `get_events` | `events::get_events` |
| `memory.list_sessions` | `list_sessions` | `events::list_sessions` |
| `memory.delete_expired` | `delete_expired` | `events::delete_expired` |
| `memory.store` | `store_memory` | `memories::store_memory` |
| `memory.get` | `get_memory` | `memories::get_memory` |
| `memory.list` | `list_memories` | `memories::list_memories` |
| `memory.consolidate` | `consolidate` | `memories::consolidate_memory` |
| `memory.delete` | `delete_memory` | `memories::delete_memory` |
| `memory.recall` | `recall` | `search::recall` |
| `memory.switch_store` | `switch_store` | `StoreManager::switch` |
| `memory.current_store` | `current_store` | `StoreManager::active_name` |
| `memory.list_stores` | `list_stores` | `StoreManager::list` |
| `memory.delete_store` | `delete_store` | `StoreManager::delete` |

### Tool handler pattern

Each tool method follows the same pattern:

```rust
#[tool(name = "memory.add_event", description = "Store an immutable conversation or blob event")]
async fn add_event(&self, Parameters(params): Parameters<AddEventParams>) -> String {
    self.run(|mgr| {
        let db = mgr.db()?;
        let p = events::InsertEventParams {
            actor_id: &params.actor_id,
            session_id: &params.session_id,
            // ... map fields
        };
        let event = events::add_event(db, &p)?;
        Ok(serde_json::to_string(&event).unwrap())
    }).await
}
```

### Helper method: `run`

A single helper on `MemoryServer` that handles the `spawn_blocking` + mutex + error conversion:

```rust
impl MemoryServer {
    async fn run<F, T>(&self, f: F) -> Result<String, String>
    where
        F: FnOnce(&mut StoreManager) -> Result<T, MemoryError> + Send + 'static,
        T: Serialize + Send + 'static,
    {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || {
            let mut mgr = store.lock().unwrap_or_else(|e| {
                tracing::warn!("mutex was poisoned by a previous panic, recovering");
                e.into_inner()
            });
            f(&mut mgr)
        })
        .await
        {
            Ok(Ok(value)) => serde_json::to_string(&value)
                .map_err(|e| format!(r#"{{"code":"internal","message":"serialization error: {e}"}}"#)),
            Ok(Err(e)) => {
                let code = match &e {
                    MemoryError::NotFound(_) => "not_found",
                    MemoryError::InvalidInput(_) | MemoryError::InvalidName(_) => "invalid_input",
                    MemoryError::ActiveStoreDeletion(_) => "invalid_input",
                    _ => "internal",
                };
                Err(serde_json::json!({"code": code, "message": e.to_string()}).to_string())
            }
            Err(join_err) => {
                tracing::error!("tool handler panicked: {join_err}");
                Err(r#"{"code":"internal","message":"internal error"}"#.into())
            }
        }
    }
}
```

**Error response format (H4)**: Tool errors return a JSON string with `code` and `message` fields. The `code` is machine-readable (`not_found`, `invalid_input`, `internal`). rmcp sets `is_error = true` on the `CallToolResult` because the handler returns `Err(...)`.

**Serialization safety (H3)**: `serde_json::to_string` uses `.map_err()` instead of `.unwrap()`. Serialization failures produce a proper error.

**Mutex poison logging**: `tracing::warn!` on recovery so it's visible in logs.

**JoinError logging**: `tracing::error!` on panic so the diagnostic is captured even if the default panic hook is replaced.

### Store management tools need `&mut StoreManager`

`switch` and `delete` need `&mut StoreManager` (not `&dyn Db`). The `run` helper already passes `&mut StoreManager`, so these tools call store methods directly.

### `memory.current_store` — no params

Uses `self.run(|mgr| { ... })` with no `Parameters<T>` extraction.
}
```

---

## Error Handling

### Error sanitization at MCP boundary

Per core-db-layer.md: "The MCP layer will sanitize errors into generic messages before sending to the client — no paths or SQL in JSON-RPC responses."

The `run` helper converts `MemoryError` to a JSON error body with `code` and `message` fields. The `code` is machine-readable for programmatic handling. The `message` uses `MemoryError::Display` which produces sanitized messages (no paths, no SQL).

Error code mapping:

| MemoryError variant | MCP error code |
|---------------------|----------------|
| `NotFound(_)` | `not_found` |
| `InvalidInput(_)` | `invalid_input` |
| `InvalidName(_)` | `invalid_input` |
| `ActiveStoreDeletion(_)` | `invalid_input` |
| All others | `internal` |

### JoinError (spawn_blocking panic)

If `spawn_blocking` panics, the `JoinError` is logged at `error` level and mapped to `{"code":"internal","message":"internal error"}` with `is_error = true`.

---

## main.rs

```rust
use local_memory_mcp::store::StoreManager;
use std::sync::{Arc, Mutex};

mod tools;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Init tracing to stderr (MCP uses stdout for JSON-RPC)
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .with_writer(std::io::stderr)
        .init();

    // Init store
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

    let server = tools::MemoryServer::new(Arc::new(Mutex::new(store_mgr)));

    // Serve over stdio
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
```

### Startup sequence (H7 — no process::exit, normal drop semantics)

1. Init tracing to stderr with env filter (default: `info`)
2. Create `StoreManager` (resolves base dir, creates if needed)
3. Open default store (creates `default.db` if new)
4. Log startup success with active store name
5. Create `MemoryServer` wrapping the store in `Arc<Mutex<_>>`
6. Start stdio transport
7. `serve_server` performs MCP initialize handshake
8. `service.waiting()` blocks until the client disconnects

**Error handling (H8)**: Startup errors print a human-readable message to stderr via `eprintln!` before propagating. The `?` operator ensures normal Rust drop semantics — `StoreManager::drop` runs and checkpoints WAL.

### Shutdown

When the MCP client disconnects (stdin closes), `service.waiting()` returns. `main` returns `Ok(())`, dropping all values in reverse declaration order. `StoreManager::drop` calls `close_active_best_effort()` which checkpoints WAL and closes the connection.

---

## Cargo.toml Changes

Add `schemars` as a direct dependency pinned to match rmcp's version:

```toml
schemars = "1"  # Must match rmcp's schemars version
```

Also add `base64` for blob_data encoding (C1):

```toml
base64 = "0.22"
```

---

## BranchFilter Mapping

The MCP `branch_filter` param is a string. Mapping to the internal enum:

```rust
fn parse_branch_filter(s: Option<&str>) -> BranchFilter<'_> {
    match s {
        None | Some("all") => BranchFilter::All,
        Some("main") => BranchFilter::MainOnly,
        Some(id) => BranchFilter::Specific(id),
    }
}
```

---

## ConsolidateAction Mapping

```rust
fn parse_consolidate_action<'a>(
    action: &ConsolidateActionType,
    new_content: Option<&'a str>,
    new_embedding: Option<&'a [f32]>,
) -> Result<ConsolidateAction<'a>, MemoryError> {
    match action {
        ConsolidateActionType::Update => {
            let content = new_content.ok_or_else(|| {
                MemoryError::InvalidInput("new_content is required for update action".into())
            })?;
            Ok(ConsolidateAction::Update { content, embedding: new_embedding })
        }
        ConsolidateActionType::Invalidate => Ok(ConsolidateAction::Invalidate),
    }
}
```

---

## Implementation Plan

| # | Task | Acceptance Criteria |
|---|------|-------------------|
| 1 | Add `schemars` and `base64` to Cargo.toml | `cargo check` passes |
| 2 | Fix H1: Add NaN/infinity validation to `memories.rs` `validate_insert_memory_params` and `validate_consolidate_params` | Existing tests pass, new test for NaN rejection |
| 3 | Create `src/tools.rs` with `MemoryServer` struct, `run` helper, enums, param structs, and all 15 tool handlers | Compiles with `cargo check` |
| 4 | Rewrite `src/main.rs` with tokio runtime, tracing, store init, and stdio server | Compiles with `cargo check` |
| 5 | Remove `#![allow(dead_code)]` from `lib.rs` | `cargo check` passes |
| 6 | Add tests: `run` helper (ok/error/panic), `parse_branch_filter`, `parse_consolidate_action`, one end-to-end tool test | `cargo test` passes |
| 7 | Run `cargo check && cargo test && cargo clippy -- -D warnings` | All pass |

---

## DAG

```
[1: Cargo.toml deps]  [2: NaN validation fix]
      │                       │
      └───────┬───────────────┘
              ▼
      [3: tools.rs]
              │
              ▼
      [4: main.rs]
              │
              ▼
      [5: Remove dead_code allow]
              │
              ▼
      [6: Tests]
              │
              ▼
      [7: Full verification]
```

Tasks 1 and 2 can run in parallel. All others are sequential.

---

## Sub-Agent Instructions

### Pre-conditions
- `cargo check` and `cargo test` pass on current main
- Read: `src/events.rs`, `src/memories.rs`, `src/search.rs`, `src/store.rs`, `src/error.rs`, `src/lib.rs`

### Step 1: Create `src/tools.rs`

Create `src/tools.rs` with:

1. Imports:
```rust
use std::sync::{Arc, Mutex};
use rmcp::{tool, tool_router, schemars};
use rmcp::handler::server::wrapper::Parameters;
use serde::Deserialize;

use crate::error::MemoryError;
use crate::events::{self, BranchFilter, InsertEventParams};
use crate::memories::{self, ConsolidateAction, InsertMemoryParams, ListMemoriesParams};
use crate::search::{self, RecallParams};
use crate::store::StoreManager;
```

2. `MemoryServer` struct with `store: Arc<Mutex<StoreManager>>` and `new()` constructor.

3. `run` helper method:
```rust
impl MemoryServer {
    pub fn new(store: Arc<Mutex<StoreManager>>) -> Self {
        Self { store }
    }

    async fn run<F, T>(&self, f: F) -> Result<String, String>
    where
        F: FnOnce(&mut StoreManager) -> Result<T, MemoryError> + Send + 'static,
        T: serde::Serialize + Send + 'static,
    {
        let store = self.store.clone();
        match tokio::task::spawn_blocking(move || {
            let mut mgr = store.lock().unwrap_or_else(|e| e.into_inner());
            f(&mut mgr)
        })
        .await
        {
            Ok(Ok(value)) => Ok(serde_json::to_string(&value).unwrap()),
            Ok(Err(e)) => Err(e.to_string()),
            Err(_) => Err("internal error".into()),
        }
    }
}
```

4. All MCP param structs (as defined in this design doc). Use `#[derive(Deserialize, schemars::JsonSchema)]`. Add `#[serde(default)]` on all optional fields.

5. Helper: `fn default_true() -> bool { true }` for `valid_only` default.

6. `#[tool_router(server_handler)]` impl block with all 15 tools. Each tool:
   - Takes `Parameters<XxxParams>` (or no params for `delete_expired` and `current_store`)
   - Calls `self.run(|mgr| { ... })` 
   - Maps MCP param struct fields to internal business logic param struct fields
   - Returns `Result<String, String>`

7. Add `pub mod tools;` to `src/lib.rs`.

### Step 2: Rewrite `src/main.rs`

Replace the placeholder with the full main function as described in the design. Use `local_memory_mcp::tools::MemoryServer` and `local_memory_mcp::store::StoreManager`.

### Step 3: Clean up dead_code

Remove `#![allow(dead_code)]` from `src/lib.rs`. If specific items still trigger warnings (e.g., test helpers), add targeted `#[allow(dead_code)]` on those items only.

### Step 4-5: Build and test

```bash
cargo build
cargo check
cargo test
cargo clippy -- -D warnings
```

### Tool implementation details

**`memory.add_event`**: Map `AddEventParams` fields to `InsertEventParams` references. Call `events::add_event(db, &p)`.

**`memory.get_event`**: Call `events::get_event(db, &params.actor_id, &params.event_id)`.

**`memory.get_events`**: Parse `branch_filter` string to `BranchFilter` enum. Map to `events::GetEventsParams`. Call `events::get_events(db, &p)`.

**`memory.list_sessions`**: Call `events::list_sessions(db, &params.actor_id, limit, offset)`.

**`memory.delete_expired`**: Call `events::delete_expired(db)`. Return `{"deleted": count}`.

**`memory.store`**: Map to `InsertMemoryParams`. For `embedding`, convert `Option<Vec<f32>>` to `Option<&[f32]>` via `.as_deref()`. Call `memories::store_memory(db, &p)`.

**`memory.get`**: Call `memories::get_memory(db, &params.actor_id, &params.memory_id)`.

**`memory.list`**: Map to `ListMemoriesParams`. Call `memories::list_memories(db, &p)`.

**`memory.consolidate`**: Parse `action` string + `new_content` + `new_embedding` into `ConsolidateAction`. Call `memories::consolidate_memory(db, &actor_id, &memory_id, &action)`.

**`memory.delete`**: Call `memories::delete_memory(db, &params.actor_id, &params.memory_id)`. Return `{"deleted": true}`.

**`memory.recall`**: Map to `RecallParams`. Call `search::recall(db, &p)`.

**`memory.switch_store`**: Call `mgr.switch(&params.name)`. Return `{"store": name}`.

**`memory.current_store`**: Call `mgr.active_name()`. Return `{"store": name}`.

**`memory.list_stores`**: Call `mgr.list()`. Return the vec directly.

**`memory.delete_store`**: Call `mgr.delete(&params.name)`. Return `{"deleted": true}`.
