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

_(Updated as we go)_
