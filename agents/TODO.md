# TODO — Work Tracking

---

## Completed

- [x] Research AgentCore Memory features and requirements
- [x] Research open-source multi-model databases with Rust libraries
- [x] Evaluate SurrealDB license (BSL 1.1) — decided to pivot to SQLite
- [x] Design main architecture (DESIGN.md) — SQLite + FTS5 + sqlite-vec
- [x] Create GitHub repository
- [x] Create agents directory with workflow and review personas
- [x] Create tracking files (this file, ADR, LESSONS_LEARNED, TIME_LOG)
- [x] Component 1: Core DB layer — design (4 review rounds, 39 findings resolved), code (23 tests), ready for code review

## In Progress

- [ ] Component 1: Core DB layer — **Phase 5 merge pending** (4 High code review findings fixed, tests pruned to 21 Critical/High paths only)

## Planned — Implementation (per-component, each goes through full workflow)

- [ ] Component 1: Core DB layer — design, review, code, review
- [ ] Component 2: Event tools — design, review, code, review
- [ ] Component 3: Memory tools — design, review, code, review
- [ ] Component 4: Search (FTS5 + vector) — design, review, code, review
- [ ] Component 5: Session tools (checkpoints, branches) — design, review, code, review
- [ ] Component 6: Store management tools — design, review, code, review
- [ ] Component 7: Namespace tools — design, review, code, review
- [ ] Component 8: MCP server — design, review, code, review
- [ ] Component 9: CI/CD — design, review, code, review
- [ ] Component 10: Installers — design, review, code, review

## Backlog

### From kiro-graph lessons learned
- [ ] Error sanitization: sanitize internal errors at MCP response boundary
- [ ] Graceful shutdown: signal handler to cleanly close SQLite before exit
- [ ] Observability: add tracing spans for connect, switch, query operations
- [ ] Disk space warning: log at startup if < 100MB free
- [ ] Install script checksums: SHA256 verification of downloaded binaries
- [ ] Input length limits: define max lengths for content, names, etc.

### From Component 1 design review (Medium/Low)
- [ ] Use ISO 8601 timestamps with UTC marker (`strftime('%Y-%m-%dT%H:%M:%SZ', 'now')`) instead of `datetime('now')`
- [ ] Add `memory.rebuild_index` tool for FTS5 desync recovery
- [ ] Add `DiskFull` error variant mapping `SQLITE_FULL`
- [ ] Connection health check in `conn()` (e.g., `SELECT 1` on stale connection)
- [ ] Configurable max store size (`LOCAL_MEMORY_MAX_SIZE_MB`, default 1GB)
- [ ] Expand `~` in `LOCAL_MEMORY_HOME` to home dir instead of rejecting
- [ ] Add `dry_run` param to `memory.delete_namespace` for cascading delete safety
- [ ] Validate embedding dimension on insert (match EMBEDDING_DIM constant)
- [ ] Add index on `memories.source_session_id`
- [ ] Input validation section in DESIGN.md for all MCP tool parameters
- [ ] `memory.import` schema validation, size limits, new UUIDs, atomic transaction
- [ ] Recursive CTE traversal: cap total visited nodes at 1000
- [ ] MCP request size limit (16MB max JSON-RPC message)
- [ ] `list()` include skipped files in warnings
- [ ] Windows long path handling (260 char limit)
- [ ] MCP `initialize` handshake behavior documentation
- [ ] Embedding dimension configurable per-store via `_meta` key
- [ ] Add minimal CI workflow before coding (not at Component 9)

### From Component 1 code review (Medium/Low)
- [ ] close_active: use take() pattern to remove store before operating on it
- [ ] list(): replace .flatten() with explicit error logging via tracing::warn!
- [ ] Remove dual module declarations from main.rs, use library crate instead
- [ ] Add doc comments to db::open, db::migrate, and module doc to store.rs
- [ ] Remove double name validation in switch → open_store
- [ ] Gate with_base_dir behind #[cfg(test)] or add validation
- [ ] Aux file deletion: check for symlinks on .db-wal and .db-shm before removing
- [ ] EMBEDDING_DIM constant not used in DDL string — interpolate or add assertion
- [ ] Add tracing::info! on successful lifecycle events (open, migrate, switch, close)
- [ ] Set 0o600 permissions on individual .db files after creation (Unix)
- [ ] Windows has_bad_prefix: also reject \\.\\ and \\?\\ prefixes

### Future features
- [ ] Local embedding model (ort + all-MiniLM-L6-v2)
- [ ] Automatic extraction (on-device LLM)
- [ ] Graph relationships between memories
- [ ] Import/export compatible with AgentCore Memory format
- [ ] Encryption at rest (sqlcipher)
- [ ] Web UI for browsing memories
