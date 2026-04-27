# Lessons Learned

Capture what went wrong, what surprised us, and what we'd do differently.

---

## From kiro-graph (predecessor project)

- **Evaluate licenses before writing code.** kiro-graph was built on SurrealDB (BSL 1.1) which restricts offering as a Database Service. Discovered this after implementation started. Lesson: read the license first.

- **Pick the storage engine that matches the access pattern.** Started with SQLite, pivoted to SurrealDB for graph edges, then pivoted back to SQLite when we realized agent memory doesn't need native graph traversal. The primary patterns are: store events, search by text/vector, retrieve by session. SQLite handles all of these.

- **Ask about deployment constraints before choosing a language.** Started with Node.js, pivoted to Rust for single-binary deployment. Ask early.

- **After resolving Critical/High review findings, always re-review.** Made substantial design changes after initial review, then tried to skip re-review. The workflow exists for a reason.

- **Check actual crate versions before writing code.** Design pinned surrealdb 2.4.0 but actual latest was 3.0.5 with breaking API changes. Always verify at implementation time.

- **Test against the real engine early.** RocksDB lock behavior (per-process, not per-handle) wasn't visible in API docs. Discovered during testing. Run integration tests early.

- **SurrealDB 3.x FTS is limited to one field per index.** Design assumed multi-field FTS indexes. Had to split into separate indexes. With SQLite FTS5, multi-column is native.

- **Don't over-engineer the data model.** kiro-graph had a single `entity` table with `node_type` field, generic `relates_to` edges. For agent memory, purpose-built tables (events, memories, checkpoints) are clearer and more efficient.

## From this project

- **`execute_batch` is NOT transactional.** Assumed it wrapped DDL in a single transaction — it doesn't. Each statement auto-commits independently. Always use explicit `conn.transaction()` for multi-statement migrations.

- **`rusqlite::Connection` is `!Send`.** This is a fundamental constraint for async MCP servers. Must design the concurrency model (spawn_blocking + std::sync::Mutex) before implementation, not after.

- **`PRAGMA locking_mode = EXCLUSIVE` doesn't acquire the lock immediately.** The lock is deferred until the first actual I/O. Must force acquisition with `BEGIN IMMEDIATE; COMMIT;` right after setting the pragma.

- **Schema changes in component designs must be reflected in DESIGN.md immediately.** The `memory_rowid` change in core-db-layer.md created a divergence that 3 of 5 reviewers flagged. Keep the canonical schema in one place.

- **Alpha crate versions can be yanked.** Document a git-rev fallback for pre-1.0 dependencies like sqlite-vec.

- **`std::mem::transmute` for FFI function pointers needs explicit safety documentation.** Pin both sides of the ABI (rusqlite + sqlite-vec versions) and document the coupling.

## From Component 1 coding

- **`#![allow(dead_code)]` is needed in both `lib.rs` and `main.rs` for a library-with-binary project.** The binary crate re-imports modules independently, so the allow in lib.rs doesn't suppress warnings in the binary. Both need it when the library has no consumers yet.

- **`conn.unchecked_transaction()` is needed for migrations.** `conn.transaction()` requires `&mut Connection`, but `migrate()` takes `&Connection` (since it's called from `open()` which already has ownership). Use `unchecked_transaction()` for this pattern.

- **`PRAGMA locking_mode = EXCLUSIVE` + `BEGIN IMMEDIATE; COMMIT;` is the correct pattern for forcing lock acquisition.** Setting the pragma alone doesn't acquire the lock — the first I/O does. The explicit transaction forces it immediately so `SQLITE_BUSY` is surfaced at open time, not at the first query.

- **`PRAGMA user_version` is NOT transactional in SQLite.** Setting it inside a transaction takes effect immediately regardless of commit/rollback. Must set it *after* a successful commit, not inside the transaction. Add a post-migration verification query to catch the case where commit fails but user_version was already updated.

- **SQLite EXCLUSIVE locking is per-process, not per-connection.** Two connections from the same process share the lock. Cross-process lock testing requires spawning a child process or using a raw connection with `BEGIN IMMEDIATE` held open.

- **Only test Critical and High code paths.** Medium/Low tests add maintenance burden without protecting against real failures. Rank every test by the severity of the code path it covers and cut the rest.

## From Component 3 (Memory tools)

- **Actor-scoping must be consistent across all operations.** The design initially had `get_memory(memory_id)` without actor_id, while `get_event(actor_id, event_id)` required it. The architecture reviewer caught this immediately. Lesson: when establishing a pattern (actor-scoped access), apply it uniformly from the start.

- **Make invalid states unrepresentable.** The initial `ConsolidateAction` enum was `Update` / `Invalidate` with separate `Option<&str>` params for content and embedding. The reviewer correctly pointed out that `Invalidate` with `new_content = Some(...)` was representable but invalid. Moving the data into the enum variants (`Update { content, embedding }`) eliminated the entire class of bugs at compile time.

- **LIKE queries need wildcard escaping.** `namespace_prefix` used `LIKE :prefix || '%'` which allowed `_` and `%` in user input to act as wildcards, bypassing namespace scoping. Always escape LIKE metacharacters with `ESCAPE '\'`.

- **Transaction ordering matters for ownership verification.** `delete_memory` initially deleted from `memory_vec` first, then from `memories` with actor_id check. If the actor check failed, the embedding was already gone. Reordering to verify ownership first (delete from `memories`) before touching `memory_vec` prevents this.

## From Component 4 (Search)

- **Constants that cross module boundaries must have a single source of truth.** The overfetch constants (`VECTOR_OVERFETCH_FACTOR`, `MAX_K_OVERFETCH`) were defined in `search.rs` and duplicated as local constants in `db.rs::search_vector` with different names. All 5 code reviewers flagged this. Define once as `pub(crate)` and import. Never duplicate constants across modules even if the values are the same today.

- **Use param structs for Db trait methods with 4+ parameters.** The initial design had `search_fts` and `search_vector` with 6 bare parameters each (actor_id, query, namespace, namespace_prefix, strategy, limit). Four reviewers flagged this as inconsistent with the existing `ListMemoriesParams`/`GetEventsParams` pattern and error-prone at call sites. Introducing `SearchFtsParams` and `SearchVectorParams` structs made the trait extensible and consistent. Apply this from the start for any method with optional filter parameters.

- **FTS5 injection prevention is the search module's responsibility, not the Db layer's.** The Db trait's `search_fts` accepts a pre-sanitized query string. The sanitization (strip operators, quote tokens, cap count) lives in `search.rs`. This keeps the Db layer simple but creates a trust boundary — any direct caller of `search_fts` must sanitize first. A `SanitizedFtsQuery` newtype would make this compile-time safe (logged to backlog).

- **sqlite-vec KNN applies post-filters, not pre-filters.** The `vec0` virtual table's `embedding MATCH ? AND k = ?` returns the top-K nearest neighbors globally, *before* any WHERE clauses on joined tables. Post-filters (actor_id, namespace, is_valid) reduce the result set below K. The mitigation is over-fetching with a capped multiplier, but this means multi-actor stores may return fewer results than requested. Document this limitation clearly.

- **Validate floating-point inputs for NaN/infinity.** Embedding vectors are `&[f32]` — they can contain NaN or infinity values that produce garbage results in distance calculations and break sort ordering. The security reviewer flagged this as "do now" priority. Always validate `is_finite()` on float inputs before passing to SQLite or sqlite-vec.

- **Hybrid search score semantics are inherently inconsistent.** FTS returns negated BM25 (unbounded), vector returns `1/(1+distance)` (0–1), and RRF returns rank-based scores (~0–0.033). There's no way to normalize these without losing information. Document the incomparability explicitly in the API and consider adding a `search_mode` field so callers know which scale they're looking at.

- **Wire new modules in `lib.rs` and `main.rs` immediately.** The binary crate (`main.rs`) has its own `mod` declarations separate from `lib.rs`. Forgetting to add `mod search;` to `main.rs` caused a compile error that wasn't caught by `cargo check` on the library crate alone. Always add to both files in the same step.

## From Component 9 (MCP Server)

- **JSON has no binary type — design blob encoding before implementation.** The design initially used `Vec<u8>` for `blob_data`, which schemars generates as an array of integers. Five reviewers flagged this as Critical/High. Base64-encoded strings are the standard for binary data over JSON-RPC. Define the encoding convention during design, not during code review.

- **Use typed enums for fixed-value string fields in MCP param structs.** String-typed fields like `event_type`, `role`, and `action` produce `{"type": "string"}` in JSON Schema with no enum constraint. LLM clients have no way to discover valid values. Rust enums with `#[derive(Deserialize, JsonSchema)]` and `#[serde(rename_all = "snake_case")]` produce proper `{"type": "string", "enum": [...]}` schemas. This eliminates runtime parsing helpers and moves validation left.

- **Never use `unwrap()` on serialization in production code.** `serde_json::to_string(&value).unwrap()` is a latent panic. Use `.map_err()` to flow serialization failures through the normal error path. Similarly, `unwrap_or_default()` silently swallows errors — propagate them instead.

- **`process::exit()` bypasses Drop.** Using `std::process::exit(1)` on startup errors skips `StoreManager::drop`, which means WAL checkpoint doesn't run. Return `Result` from `main` instead — normal Rust drop semantics handle cleanup.

- **Expand MCP tool descriptions for LLM consumption.** One-line descriptions are insufficient. LLMs need 2-4 sentences explaining: purpose, required vs optional parameters, valid enum values, and return shape. The tool description is the primary signal an LLM uses to decide when and how to call a tool.

- **Hand-formatted JSON strings are fragile.** Using `format!()` to construct JSON error responses risks malformed output if the interpolated values contain quotes or backslashes. Use `serde_json::json!()` or a typed struct with `Serialize` for all JSON construction.
