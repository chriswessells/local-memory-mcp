# Integration & End-to-End Tests — Detailed Design

## Problem

All 101 existing tests are unit tests exercising individual layers in isolation. Nothing tests the full path: MCP tool param deserialization → `self.run()` closure → business logic validation → Db trait SQL → SQLite → response serialization → JSON output.

## Solution

Two test layers:

1. **Integration tests** (`tests/integration.rs`) — call `MemoryServer` tool methods directly, verify JSON responses through the full internal stack.
2. **E2E tests** (`tests/e2e.rs`) — spawn the binary, communicate over stdio with MCP JSON-RPC protocol, verify real deployment path.

---

## Integration Test Architecture

### Invocation Pattern

The `ServerHandler::call_tool` requires a `RequestContext<RoleServer>` with a `Peer` backed by a real transport channel — too complex to construct in tests. Instead, call tool methods directly with `Parameters(params)` where params are deserialized from `serde_json::Value` to test the full serde path:

```rust
use rmcp::handler::server::wrapper::Parameters;

// Deserialize from JSON Value → typed params (tests serde)
// Then call tool method directly (tests run() + business logic + SQL)
async fn call(server: &MemoryServer, json: serde_json::Value) -> serde_json::Value {
    // The specific tool method is called by the test, not dispatched dynamically.
    // This is acceptable because dispatch is rmcp's responsibility, not ours.
}
```

Each test deserializes params from a `serde_json::Value` via `serde_json::from_value::<T>()`, wraps in `Parameters(params)`, calls the tool method, and parses the `Result<String, String>` response back to `serde_json::Value`.

### Shared Test Helpers (`tests/common/mod.rs`)

```rust
mod common;

// Setup: creates TempDir + MemoryServer with default store open
fn setup() -> (TempDir, MemoryServer);

// Parse tool Ok response to JSON Value
fn parse_ok(result: Result<String, String>) -> serde_json::Value;

// Parse tool Err response to JSON Value, verify error code
fn parse_err(result: Result<String, String>, expected_code: &str) -> serde_json::Value;
```

---

## E2E Test Architecture

### MCP Protocol Compliance

The E2E tests follow the full MCP handshake:

1. **Send `initialize` request** with `protocolVersion`, `capabilities`, `clientInfo`
2. **Receive `initialize` response** with server capabilities
3. **Send `notifications/initialized`** notification (no id, no response expected)
4. **Now `tools/call` and `tools/list` are valid**
5. **Close stdin** to signal shutdown → verify process exits with code 0

### JSON-RPC Framing

MCP over stdio uses **newline-delimited JSON** (confirmed by reading rmcp 1.5.0 `JsonRpcMessageCodec` source — the `Encoder` appends `\n`, the `Decoder` splits on `\n`). Each message is a single JSON object followed by `\n`.

- Write: `serde_json::to_string(&msg)` + `\n` + flush
- Read: `BufReader::read_line()`, parse as JSON, match by `id` field
- Discard any server-initiated notifications (messages without `id`)

### Timeouts

- Server startup (first response): **5 seconds**
- Per tool call: **2 seconds**
- Overall test: **15 seconds**
- Use `tokio::time::timeout` wrapping each operation

### Binary Reference

```rust
env!("CARGO_BIN_EXE_local-memory-mcp")  // resolved by cargo test
```

### Child Process Cleanup

Use a Drop guard struct that kills the child process if the test panics before clean shutdown:

```rust
struct ServerProcess { child: Child }
impl Drop for ServerProcess {
    fn drop(&mut self) { let _ = self.child.kill(); }
}
```

---

## Integration Test Cases

### 1. `test_event_lifecycle` (Critical)

1. `memory.create_event` (conversation) → verify `id`, `actor_id`, `created_at` in response
2. `memory.get_event` → verify same event
3. `memory.list_events` → verify event in list
4. `memory.list_sessions` → verify session with event_count=1
5. `memory.create_event` with `expires_at: "2000-01-01T00:00:00Z"` (far past)
6. `memory.delete_expired_events` → verify `{"deleted": 1}`

### 2. `test_memory_lifecycle` (Critical)

1. `memory.create_memory_record` → verify `id`, `is_valid: true`
2. `memory.get_memory_record` → verify same memory
3. `memory.list_memory_records` → verify memory in list
4. `memory.update_memory_record` (update) → verify new memory, old invalid
5. `memory.list_memory_records` (valid_only=true) → only new memory
6. `memory.delete_memory_record` → `{"deleted": true}`
7. `memory.get_memory_record` on deleted → not_found error

### 3. `test_recall_fts` (High)

1. Store 3 memories with distinct content
2. `memory.retrieve_memory_records` with text query → verify correct match with score > 0

### 4. `test_graph_lifecycle` (Critical)

1. Store 2 memories
2. `graph.create_edge` → verify `id`, `label`
3. `graph.get_neighbors` → verify connected memory
4. `graph.traverse` → verify depth=1 node
5. `graph.update_edge` → verify label changed
6. `graph.delete_edge` → `{"deleted": true}`
7. `graph.list_labels` → verify labels returned
8. `graph.get_stats` → verify total_edges count

### 5. `test_store_isolation` (Critical)

1. Store a memory in default store
2. `store.current` → verify "default"
3. `store.switch` to "other"
4. `memory.list_memory_records` → verify empty
5. Store a memory in "other"
6. `store.switch` back to "default"
7. `memory.list_memory_records` → verify only original memory
8. `store.list` → verify both listed
9. `store.delete` "other" → deleted

### 6. `test_actor_isolation` (Critical)

1. Store two memories as "alice"
2. `memory.get_memory_record` as "bob" with alice's memory ID → not_found
3. Store memory as "bob"
4. `memory.list_memory_records` as "alice" → only alice's two memories
5. Add edge between alice's two memories
6. `graph.get_neighbors` as "bob" with alice's memory ID → empty

### 7. `test_blob_event_roundtrip` (High)

1. `memory.create_event` with event_type=blob, base64 blob_data
2. `memory.get_event` → verify blob_data matches original base64

### 8. `test_error_responses` (High)

1. `memory.get_event` nonexistent → `{"code":"not_found",...}`
2. `memory.create_memory_record` empty content → `{"code":"invalid_input",...}`
3. `graph.create_edge` self-edge → `{"code":"invalid_input",...}`

---

## E2E Test Cases

### 1. `test_e2e_mcp_lifecycle` (Critical)

1. Spawn binary with temp `LOCAL_MEMORY_HOME`
2. Send `initialize` → verify capabilities (tools listed)
3. Send `notifications/initialized`
4. Send `tools/list` → verify 22 tools present, spot-check one tool's `inputSchema` has expected required fields
5. Send `tools/call` `memory.create_event` → verify success response
6. Send `tools/call` `memory.create_memory_record` → verify success (memory family)
7. Send `tools/call` `memory.retrieve_memory_records` with query → verify result (search family)
8. Send `tools/call` `graph.create_edge` (using memory IDs from steps 5-6) → verify success (graph family)
9. Send `tools/call` with invalid params → verify `isError: true` in result (not a JSON-RPC error)
10. Close stdin → verify exit code 0

### 2. `test_e2e_stderr_logging` (High)

1. Spawn binary with `RUST_LOG=info` explicitly set
2. Send initialize + initialized + one tool call
3. Read stderr → verify non-empty (tracing output present)
4. Verify stdout lines are all valid JSON (no log contamination)
5. Close stdin

---

## What We Do NOT Test

- **Vector search** — requires 384-dim embeddings, covered by unit tests
- **Hybrid RRF search** — unit tests cover fusion logic
- **Platform-specific paths** — CI matrix responsibility
- **MCP transport edge cases** — rmcp's responsibility
- **Concurrent tool calls** — single-process EXCLUSIVE locking, mutex serializes access

---

## Dependencies

Add `process` feature to tokio for E2E tests. In Cargo.toml:

```toml
[dev-dependencies]
tokio = { version = "1", features = ["process", "io-util"] }
```

Cargo merges features from `[dependencies]` and `[dev-dependencies]`, so the existing `macros`, `rt-multi-thread`, `sync` features are preserved.

No other new dependencies. `tempfile` already in dev-dependencies.

---

## File Structure

```
tests/
├── common/
│   └── mod.rs          # Shared helpers: setup(), parse_ok(), parse_err(), e2e helpers
├── integration.rs      # 8 integration test cases
└── e2e.rs              # 2 E2E test cases
```

---

## Implementation Plan

### Task 1: Shared helpers + Cargo.toml update
- Create `tests/common/mod.rs` with `setup()`, `parse_ok()`, `parse_err()`
- Add `tokio = { version = "1", features = ["process", "io-util"] }` to `[dev-dependencies]`
- Run `cargo check --tests`

### Task 2: Integration tests
- Create `tests/integration.rs` with all 8 test cases
- Each test uses `#[tokio::test]` and calls tool methods via `Parameters(serde_json::from_value(json).unwrap())`
- Run `cargo test --test integration`

### Task 3: E2E tests
- Create `tests/e2e.rs` with both E2E test cases
- Implement MCP JSON-RPC helpers: `send_request()`, `read_response()`, `mcp_initialize()`, `ServerProcess` drop guard
- Run `cargo test --test e2e`

### Task 4: Verify all tests pass together
- Run `cargo test` (all unit + integration + E2E)
- Run `cargo clippy -- -D warnings`

---

## DAG

```
Task 1 ──► Task 2 ──► Task 4
       └──► Task 3 ──┘
```

Task 1 (helpers) must come first. Tasks 2 (integration) and 3 (E2E) can run in parallel since they share helpers but don't depend on each other. Task 4 (verify) depends on both.

---

## Sub-Agent Instructions

### Task 1: Shared helpers + Cargo.toml

1. Add to `Cargo.toml` under `[dev-dependencies]`:
   ```toml
   tokio = { version = "1", features = ["process", "io-util"] }
   ```

2. Create `tests/common/mod.rs`:
   ```rust
   use std::sync::{Arc, Mutex};
   use tempfile::TempDir;
   use local_memory_mcp::store::StoreManager;
   use local_memory_mcp::tools::MemoryServer;

   pub fn setup() -> (TempDir, MemoryServer) {
       let dir = TempDir::new().unwrap();
       let mut mgr = StoreManager::with_base_dir(dir.path().to_path_buf()).unwrap();
       mgr.open_default().unwrap();
       let server = MemoryServer::new(Arc::new(Mutex::new(mgr)));
       (dir, server)
   }

   pub fn parse_ok(result: Result<String, String>) -> serde_json::Value {
       let s = result.expect("expected Ok response");
       serde_json::from_str(&s).expect("response is not valid JSON")
   }

   pub fn parse_err(result: Result<String, String>, expected_code: &str) -> serde_json::Value {
       let s = result.expect_err("expected Err response");
       let v: serde_json::Value = serde_json::from_str(&s).expect("error is not valid JSON");
       assert_eq!(v["code"].as_str().unwrap(), expected_code);
       v
   }
   ```

3. Run `cargo check --tests` to verify.

### Task 2: Integration tests

1. Create `tests/integration.rs`.
2. Add `mod common;` at the top.
3. Implement all 8 test cases from the design. Each test:
   - Calls `common::setup()` to get a `MemoryServer`
   - Deserializes params from `serde_json::json!({...})` via `serde_json::from_value::<ParamType>(json).unwrap()`
   - Wraps in `Parameters(params)` and calls the tool method: `server.add_event(Parameters(params)).await`
   - Parses response with `common::parse_ok()` or `common::parse_err()`
   - Asserts on JSON fields

4. Import patterns:
   ```rust
   mod common;
   use common::{setup, parse_ok, parse_err};
   use rmcp::handler::server::wrapper::Parameters;
   use serde_json::json;
   ```

5. **Parameterless tools**: `delete_expired()`, `current_store()`, `list_stores()` take no `Parameters` — call directly: `server.delete_expired().await`. All other tools use `Parameters(serde_json::from_value::<ToolParamType>(json).unwrap())` where the type is inferred from the method signature.

6. **Private param types**: The param structs in `tools.rs` are private. Do NOT try to import them. Instead, let Rust infer the type from the method signature:
   ```rust
   // Type is inferred from server.add_event's signature
   let result = server.add_event(Parameters(serde_json::from_value(json!({
       "actor_id": "a1", "session_id": "s1", "event_type": "conversation",
       "role": "user", "content": "hello"
   })).unwrap())).await;
   ```

7. For `test_event_lifecycle`, `test_memory_lifecycle`, `test_graph_lifecycle`, `test_store_isolation`, `test_actor_isolation`: these are multi-step tests that build on previous results (e.g., use the `id` from step 1 in step 2).

6. For `test_blob_event_roundtrip`: use `base64::engine::general_purpose::STANDARD.encode(b"hello")` for the blob_data.

7. For `test_error_responses`: verify the error JSON has `"code"` and `"message"` fields.

8. Run `cargo test --test integration`.

### Task 3: E2E tests

1. Create `tests/e2e.rs`.
2. Add `mod common;` at the top (for potential shared helpers, though E2E mostly has its own).
3. Implement `ServerProcess` struct with Drop guard:
   ```rust
   struct ServerProcess {
       child: tokio::process::Child,
       stdin: tokio::io::BufWriter<tokio::process::ChildStdin>,
       stdout: tokio::io::BufReader<tokio::process::ChildStdout>,
   }
   impl Drop for ServerProcess {
       fn drop(&mut self) { let _ = self.child.start_kill(); }
   }
   ```

4. Implement helpers:
   - `spawn_server(base_dir: &Path) -> ServerProcess` — spawns binary with `LOCAL_MEMORY_HOME` env, `RUST_LOG=info`
   - `send_request(proc, id, method, params) -> ()` — writes newline-delimited JSON-RPC to stdin
   - `send_notification(proc, method, params) -> ()` — writes JSON-RPC notification (no id)
   - `read_response(proc, expected_id) -> Value` — reads lines, skips notifications, returns response matching id. Uses `tokio::time::timeout(Duration::from_secs(2), ...)`. If `read_line` returns 0 bytes (EOF), return error immediately — the server has crashed.
   - `mcp_initialize(proc) -> Value` — sends initialize request + initialized notification, returns capabilities

5. Implement `test_e2e_mcp_lifecycle`:
   - Spawn server
   - `mcp_initialize`: send exact JSON:
     ```json
     {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"test","version":"0.1.0"}}}
     ```
     Then send notification (no id):
     ```json
     {"jsonrpc":"2.0","method":"notifications/initialized"}
     ```
   - `tools/list` → assert 22 tools present (NOTE: update count if tools change), spot-check `memory.create_event` has `actor_id` in inputSchema
   - `tools/call` `memory.create_event` → response is `{"jsonrpc":"2.0","id":N,"result":{"content":[{"type":"text","text":"..."}]}}`. Parse `result.content[0].text` as JSON, verify has `id`
   - `tools/call` `memory.create_memory_record` → verify has `id` (memory family)
   - `tools/call` `memory.retrieve_memory_records` with query → verify result (search family)
   - `tools/call` `graph.create_edge` (using memory IDs from previous steps) → verify has `id` (graph family)
   - `tools/call` with invalid params (empty actor_id) → verify `result.isError` is `true` and `result.content[0].text` contains error JSON (NOT a JSON-RPC error — MCP returns tool errors as successful responses with isError flag)
   - Drop ServerProcess (closes stdin, kills if needed)

6. Implement `test_e2e_stderr_logging`:
   - Spawn server with `RUST_LOG=info`
   - Initialize + one tool call
   - Drain stderr concurrently: spawn a `tokio::spawn` task that reads stderr into a `Vec<String>` while the main test flow runs on stdout. This prevents pipe buffer deadlock.
   - Assert stderr is non-empty
   - Assert each stdout line is valid JSON
   - Drop ServerProcess

7. Run `cargo test --test e2e`.

### Task 4: Final verification

1. Run `cargo test` (all tests together)
2. Run `cargo clippy -- -D warnings`
3. Verify test count increased (should be ~101 unit + 8 integration + 2 E2E = ~111)
