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
