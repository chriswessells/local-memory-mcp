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
- [x] Component 1: Core DB layer — design (4 review rounds, 39 findings resolved), code (21 tests), code review (4 High fixed), merged to main

- [x] Component 2: Event tools — design (2 review rounds), code (15 tests), code review (0 High), merged

- [x] Component 3: Memory tools — design (2 review rounds, 6 High resolved), code (23 tests), code review (1 High fixed), merged

## In Progress

_(none)_

## Planned — Implementation (per-component, each goes through full workflow)

- [ ] Component 1: Core DB layer — design, review, code, review
- [ ] Component 2: Event tools — design, review, code, review
- [ ] Component 3: Memory tools — design, review, code, review
- [x] Component 4: Search (FTS5 + vector) — design (2 review rounds, 6 High resolved), code (22 tests), code review pending
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

### From Component 2 design review (Medium/Low)
- [ ] Use `serde_bytes` for `blob_data` serialization (avoid JSON integer arrays)
- [ ] Add `metadata_filter` parameter to `get_events` Db trait (reserved slot, error if used initially)
- [ ] Add CHECK constraints on `event_type IN ('conversation','blob')` and `role` values
- [ ] Add immutability trigger on events table (`BEFORE UPDATE ... RAISE(ABORT)`)
- [ ] Batch `delete_expired_events` with LIMIT to bound lock-hold time
- [ ] Use named SQL parameters (`:name`) for all dynamic queries in get_events
- [ ] Custom `Debug` impl for `Event` that redacts blob_data
- [ ] Restrict `actor_id`/`session_id` to printable ASCII (reject control chars)
- [ ] Validate metadata as JSON object with max depth (10) and max keys (100)
- [ ] Enforce `expires_at` must be in the future
- [ ] Handle cascading deletes for checkpoints/branches referencing expired events
- [ ] Log deleted event IDs at debug level in `delete_expired_events`
- [ ] Add `serde(rename)` for AgentCore field name compatibility in MCP response DTOs
- [ ] Document blob_data base64 encoding convention for MCP JSON transport
- [ ] Document MCP tool response envelope shapes (`{"event": {...}}`, `{"events": [...]}`)
- [ ] Implement cursor-based pagination at MCP tool layer using `after` as cursor

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

### From Component 3 design review (Medium/Low)
- [ ] Extract shared validation helpers to `src/validation.rs` (avoid duplication across modules)
- [ ] Split `db.rs` into module directory (`db/mod.rs`, `db/events.rs`, `db/memories.rs`)
- [ ] Use named column access in `row_to_memory` instead of positional indices
- [ ] Add optional `new_metadata` to consolidate Update, or document metadata not inherited
- [ ] Document consolidation embedding behavior: old embedding always deleted
- [ ] Add database size quota check before inserts (`MAX_STORE_SIZE_BYTES`)
- [ ] Unify permission hardening in `with_base_dir` (currently skips `chmod 0o700`)
- [ ] Log warning when `LOCAL_MEMORY_SYNC=normal` downgrades durability
- [ ] Add startup consistency check: valid memories missing `memory_vec` rows → warn
- [ ] Verify `sqlite-vec` vec0 virtual tables participate in SQLite transaction rollback
- [ ] Add `memory_fts` rebuild procedure (`INSERT INTO memory_fts(memory_fts) VALUES('rebuild')`)
- [ ] LIKE escaping: document that namespace_prefix `%` and `_` are escaped before query

### From Component 3 code review (Medium/Low)
- [ ] Deduplicate INSERT SQL in insert_memory (extract helper or always use transaction)
- [ ] Consolidate Invalidate: delete embedding from memory_vec for consistency with Update
- [ ] Use `row.get::<_, bool>()` instead of `row.get::<_, i32>() != 0` in row_to_memory
- [ ] Add `#[serde(skip_serializing_if = "Option::is_none")]` to Memory optional fields
- [ ] Add max-length validation on `memory_id` and `source_session_id` inputs
- [ ] Add `validate_max_len` for `actor_id` in get/consolidate/delete (consistency with store)
- [ ] Document `unchecked_transaction` invariant: safe under EXCLUSIVE locking mode
- [ ] Add comment on consolidate_memory: metadata/source_session_id intentionally not inherited

### From Component 4 design review (Medium/Low)
- [ ] CJK text handling: FTS5 sanitizer splits on whitespace only, CJK has no word boundaries — document limitation or add character-class splitting
- [ ] Score semantics vary by search mode — consider adding `search_mode` field to SearchResult or normalizing scores to 0.0–1.0
- [ ] Distance-to-similarity formula `1/(1+d)` is not cosine similarity — document clearly or use `1 - L2²/2` for normalized embeddings
- [ ] FTS5 sanitizer: split on hyphens too (not just whitespace) so "long-term" matches FTS5 unicode61 tokenization
- [ ] Extract `QueryBuilder` helper for dynamic SQL with optional WHERE clauses (used in get_events, list_memories, search_fts, search_vector)
- [ ] Extract `MemoryFilter` struct to share filter fields across list_memories, search_fts, search_vector
- [ ] Add embedding dimension `debug_assert_eq!` in search_vector Connection impl for defense-in-depth
- [ ] Verify query plans with `EXPLAIN QUERY PLAN` for FTS5 join and vector CTE during implementation
- [ ] Add `namespace_prefix_like(prefix: &str) -> String` helper to avoid duplicating escape_like + `%` append
- [ ] RRF HashMap clones full Memory structs — consider using indices for large result sets
- [ ] FTS5 content-sync crash recovery: add `memory.rebuild_index` tool (already in backlog)
- [ ] sqlite-vec KNN blob binding: add cross-platform integration test (especially Windows CI)
- [ ] Add `progress_handler` on SQLite connection for query timeout (server-wide concern)
- [ ] Disk-full during FTS5 query: map SQLITE_FULL to specific error variant
- [ ] WAL checkpoint failure in switch(): use best-effort close to avoid trapping user on full disk
- [ ] Document that changing embedding dimensions requires schema v2 migration + re-embedding

### From Component 4 design review round 2 (Medium/Low)
- [ ] Introduce `SanitizedFtsQuery` newtype to prevent unsanitized FTS queries at Db trait boundary
- [ ] Add `test_search_vector_namespace_filter` and `test_search_vector_valid_only` for filter parity with FTS tests
- [ ] RRF: use `or_insert_with` instead of `or_insert` to avoid eager Memory clone
- [ ] Add comment on `sanitize_fts_query`: hyphen stripping is intentional, aligned with FTS5 unicode61 tokenizer
- [ ] `search_vector` Db trait doc comment: clarify returns raw L2 distance, callers must convert to similarity
- [ ] Use structured tracing fields in hybrid fallback warn log (actor_id, query_len, vec_results count)
- [ ] Document VECTOR_OVERFETCH_FACTOR / MAX_K_OVERFETCH interaction formula in constant doc comments
- [ ] Add `dist.max(0.0)` guard in vector-only score conversion for defensive coding
- [ ] Add RRF test case for completely disjoint lists (no overlap)

### Future features
- [ ] Local embedding model (ort + all-MiniLM-L6-v2)
- [ ] Automatic extraction (on-device LLM)
- [ ] Graph relationships between memories
- [ ] Import/export compatible with AgentCore Memory format
- [ ] Encryption at rest (sqlcipher)
- [ ] Web UI for browsing memories
