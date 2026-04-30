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

- [x] Component 4: Search (FTS5 + vector) — design (2 review rounds, 6 High resolved), code (22 tests), code review (1 High fixed + 1 Medium NaN validation), merged

- [x] Component 9: MCP server — design (1 Critical + 11 High resolved), code (15 tools, 4 tests), code review (2 fixes: error propagation, JSON escaping), merged

- [x] Component 5: Knowledge graph — design (1 Critical + 2 High resolved), code (26 tests), code review (1 Critical + 1 High fixed), merged

- [x] Component 7: Store management tools — implemented as part of Component 9 MCP server (switch_store, current_store, list_stores, delete_store in tools.rs; StoreManager in store.rs), no separate design/review cycle needed

## In Progress

- [x] Component 12: Integration & E2E tests — design (2 review rounds, 2 Critical + 11 High resolved), artifact review complete
  - [x] Task 1: Shared helpers + Cargo.toml update (`tests/common/mod.rs`, tokio dev-dep)
  - [x] Task 2: Integration tests — 10 test cases in `tests/integration.rs`
    - [x] test_event_lifecycle
    - [x] test_memory_lifecycle
    - [x] test_recall_fts
    - [x] test_graph_lifecycle
    - [x] test_store_isolation
    - [x] test_actor_isolation
    - [x] test_blob_event_roundtrip
    - [x] test_error_responses
    - [x] test_namespace_lifecycle (Component 8)
    - [x] test_session_lifecycle (Component 6)
  - [x] Task 3: E2E tests — 2 test cases in `tests/e2e.rs`
    - [x] test_e2e_mcp_lifecycle
    - [x] test_e2e_stderr_logging
  - [x] Task 4: Final verification (cargo test + cargo clippy)

- [x] Bug fix: Graceful shutdown — signal handler to cleanly close SQLite before exit (design: `design/graceful-shutdown.md`)
  - [x] Research root cause (POSIX locks released by OS on death; real issue is missing WAL checkpoint/optimize)
  - [x] Design (`design/graceful-shutdown.md`)
  - [x] Code (add `shutdown_signal()` + `tokio::select!` + explicit `close_active()` in `main.rs`)
  - [x] Verify (`cargo check`, `cargo test`, `cargo clippy`, manual SIGTERM test)

## Planned — Implementation (per-component, each goes through full workflow)

- [x] Component 6: Session tools — design (2 review rounds, 2 High resolved), code (24 tests), code review (0 Critical/0 High), merged
- [x] Component 8: Namespace tools — design (2 review rounds, 1 Critical + 4 High resolved), code (11 tests, pruned to High/Critical paths only), merged
- [ ] Component 9: MCP server — ✅ done (moved to Completed)
- [x] Component 10: CI/CD — design (2 Critical + 6 High resolved in design review), code (ci.yml + release.yml), code review (3 High fixed: release atomicity, artifact verification, per-job permissions), merged
- [x] Component 11: Installers — design (1 Critical + 7 High resolved in design review), code (install.sh), code review (3 High fixed: wget TLS, tar path restriction), merged

## Backlog

### From kiro-graph lessons learned
- [ ] Error sanitization: sanitize internal errors at MCP response boundary
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

### From Component 4 code review (Medium/Low)
- [ ] Extract shared validation helpers (validate_non_empty, validate_max_len) to src/validation.rs
- [ ] Magic column index 11 for score/distance — define constant or use named column access
- [ ] Deduplicate mem_params test helper across db.rs, memories.rs, search.rs
- [ ] Extract shared SQL filter builder for namespace/namespace_prefix/strategy (used in list_memories, search_fts, search_vector)
- [ ] Add adversarial FTS injection tests (content:secret, hello*, NEAR(a b), a AND b)
- [ ] Add search_mode field to SearchResult so callers can detect hybrid→vector-only fallback
- [ ] Add validate_max_len for actor_id in memories.rs get/consolidate/delete for consistency

### From Component 9 design review (Medium/Low)
- [ ] Error messages echo user-supplied IDs in NotFound — use generic messages
- [ ] ConnectionFailed/InvalidPath errors may leak filesystem paths — sanitize at MCP boundary
- [ ] Duplicated validation helpers across modules — extract to shared module
- [ ] schemars version coupling with rmcp — consider pinning as direct dep
- [ ] Split tools.rs into submodules when >500 lines (tools/events.rs, tools/memories.rs, etc.)
- [ ] Shutdown drop ordering — add explicit drop after service.waiting()
- [ ] `metadata` as String vs serde_json::Value for better LLM usability
- [ ] AgentCore Memory parameter naming divergences — document in descriptions
- [ ] `valid_only` default not reflected in schemars schema
- [ ] `actor_id` required on consolidate/get/delete diverges from AgentCore
- [ ] Platform-specific base dir doesn't follow XDG/Apple conventions
- [ ] Add `LOCAL_MEMORY_SYNC=normal` warning log

### From Component 9 code review (Medium/Low)
- [ ] Define ToolError struct for typed error responses instead of hand-formatted JSON
- [ ] Introduce EventResponse DTO for blob encoding instead of post-hoc JSON patching
- [ ] Add pre-decode length check on base64 blob_data before decoding
- [ ] Clamp all limit params to MAX_PAGE_LIMIT at tool handler level (not just recall)
- [ ] Add explicit WAL checkpoint on shutdown (signal handler or post-service.waiting())
- [ ] Add request-level timeout on spawn_blocking (tokio::time::timeout)
- [ ] Use named constants for default pagination values instead of magic numbers
- [ ] Add integration tests for MCP tool handlers (add_event blob path, store_memory, recall)
- [ ] Add crate-level doc comment to lib.rs

### From Component 5 design review (Medium/Low)
- [ ] Properties JSON nesting depth guard (max 10 levels) to prevent stack overflow in serde_json
- [ ] Label validation: reject control characters (bytes < 0x20) in edge labels
- [ ] Duplicate edge detection: consider UNIQUE constraint on (from_memory_id, to_memory_id, label) or upsert semantic
- [ ] Consolidation edge orphaning: add mechanism to discover/re-link edges when memory is consolidated
- [ ] graph_stats most_connected query is O(n) full scan — optimize if edge count exceeds 10K
- [ ] Expose `graph.get_edge` as MCP tool (currently internal-only Db method)
- [ ] Traverse: return `truncated: bool` when max_visited cap is hit
- [ ] insert_edge: wrap existence check + INSERT in unchecked_transaction for defense-in-depth
- [ ] Traverse CTE: add LIMIT 1000 to final SELECT as belt-and-suspenders (already in design)
- [ ] CASCADE delete: include edges_removed count in memory.delete response
- [ ] Add `updated_at` column to knowledge_edges schema to avoid future migration
- [ ] Parse traverse path JSON with serde_json::from_str and map errors to QueryFailed

### From Component 10 design review (Medium/Low)
- [ ] Add `cargo audit` step to CI or as a separate scheduled workflow
- [ ] Add SLSA provenance attestation (`actions/attest-build-provenance`) when project gains users
- [ ] Add tag-version vs Cargo.toml consistency check in release workflow
- [ ] Add `key: ci` to CI workflow's rust-cache to namespace away from release caches
- [ ] Document that Linux aarch64 release binary is cross-compiled but not tested in CI
- [ ] Add CI timeout note for E2E tests if flakiness appears (split into separate step)
- [ ] Add branch protection on `main` requiring CI status check
- [ ] Pin `cross` Docker images for fully reproducible aarch64-linux builds

### From Component 11 code review (Medium/Low)
- [ ] Add VERSION env var for version pinning in install.sh
- [ ] Add specific Darwin-x86_64 case with targeted error message
- [ ] Add download timeouts (--connect-timeout, --max-time) to curl/wget
- [ ] Add HUP signal to cleanup trap
- [ ] Add --help flag to install.sh
- [ ] Add comment linking tarball name pattern to release.yml
- [ ] Validate INSTALL_DIR is an absolute path

### From Component 10 code review (Medium/Low)
- [ ] Add `--locked` to `cargo clippy` and `cargo test` in ci.yml for consistency with release
- [ ] Split E2E tests into separate CI step for better diagnostic visibility
- [ ] Add `Swatinem/rust-cache` to release workflow with `key: release-${{ matrix.target }}`
- [x] Update GitHub Actions to Node.js 24 runtime (checkout v6.0.2, rust-cache v2.9.1, upload-artifact v7.0.1, download-artifact v8.0.1) — done 2026-04-28
- [ ] Pin `dtolnay/rust-toolchain` to SHA instead of branch name
- [ ] Verify `cross` 0.2.5 compatibility with current `ubuntu-latest` runner
- [ ] Add step names to ci.yml cargo run steps for Actions UI readability
- [ ] Add top-of-file comments linking to design/ci-cd.md rationale

### From graceful shutdown design review (Medium/Low)
- [ ] Add WAL checkpoint on startup after unclean shutdown (`PRAGMA wal_checkpoint(TRUNCATE)` in `db::open()`)
- [ ] Add automated e2e test for shutdown path (spawn binary, send SIGTERM, assert WAL cleanup)
- [ ] No second-signal force-quit (register handler for immediate exit on second signal)
- [ ] No MCP-level graceful close notification before transport drop

### From graceful shutdown code review (Medium/Low)
- [ ] No shutdown timeout on hung cleanup (wrap in `tokio::time::timeout`)
- [ ] `shutdown_signal()` is untestable/untested (extract to lib crate or add e2e test)
- [ ] Gate `with_base_dir()` behind `#[cfg(test)]` to prevent production use without security checks

### From Component 6 design review (Medium/Low)
- [ ] create_branch: document EXCLUSIVE lock dependency in comment (A4)
- [ ] parent_branch actor cross-check: verify parent branch root event also belongs to actor_id (A5)
- [ ] metadata wire format: consider deserializing to serde_json::Value in Checkpoint/Branch structs instead of raw JSON string (A6)
- [ ] Use c.is_control() in validate_no_control_chars (I1 — already applied to design; enforce in code)
- [ ] Verify dotted tool names work in all target Kiro/MCP clients (I2)
- [ ] Orphan branches on event TTL expiry: add to delete_expired_events to prevent dangling root_event_id references, or document behavior in ADR (A3/R4)
- [ ] Map SQLITE_FULL to user-friendly "database is out of space" error (R2)
- [ ] Document unchecked_transaction commit/rollback flow explicitly (R3)
- [ ] Add tracing spans to sessions.rs public functions (R5)
- [ ] Map SQLITE_BUSY to MemoryError::StoreLocked in new session methods (R6)
- [ ] Consolidate MAX_CHECKPOINT_NAME_LEN / MAX_BRANCH_NAME_LEN to shared constant (M4)
- [ ] Add doc comment to sessions.rs clarifying scope vs events.rs (M5)
- [ ] UUID format validation on event_id and parent_branch_id fields (S3)
- [ ] Ensure serde_json parse errors are never propagated as-is in session tool handlers (S4)
- [ ] Document that session tool handlers log no user-supplied content (S5)

### Agentic Coding Workflow — vendor-agnostic improvements

See `design/DESIGN.md` § "Agentic Coding Workflow — Research" for full rationale, primitives, and sources. Goal: keep prompts as repo-visible, schema-bound, version-controlled artifacts that any harness (Claude Code, Kiro CLI, Cursor, Codex, Aider, Gemini CLI, etc.) can drive via shell scripts. Vendor dotfolders become thin adapters, not the source of truth.

#### Tier 1 — Make agents executable (vendor-neutral)
- [ ] Add `AGENTS.md` at repo root (entry point for Codex, Cursor, Aider, Kiro, Windsurf, Devin, Gemini CLI, etc.)
- [ ] Symlink `CLAUDE.md` → `AGENTS.md` (Claude Code does not yet auto-load AGENTS.md)
- [ ] Restructure `agents/` into `agents/personas/`, `agents/workflows/`, `agents/schemas/`
- [ ] Add YAML frontmatter (`name`, `version`, `description`, `output_schema`) to each `agents/personas/*_review.md`
- [ ] Define `agents/schemas/finding.schema.json` — JSON Schema for review findings (severity enum, location, finding, remediation, evidence)
- [ ] Write `scripts/review.sh` — invoke one persona on one input, emit schema-conformant JSON; model selected via `LLM_MODEL` env var
- [ ] Write `scripts/design-review.sh` — parallel fan-out of all 5 personas via `xargs -P5`, pipe through aggregator
- [ ] Write `scripts/code-review.sh` — same, against a diff
- [ ] Write `scripts/aggregate-findings.py` — dedupe, severity-normalize, suppress pre-existing-code findings (rules from `PERSONA_IMPROVEMENTS.md`)
- [ ] Write `scripts/close-component.sh` — gate that requires TODO/TIME_LOG/ADR/LESSONS_LEARNED updates before marking a component done
- [ ] Add `justfile` aliasing common tasks (`just review`, `just design-review`, `just code-review`, `just close component=…`, `just check`)
- [ ] Write `scripts/render-adapters.sh` — generate `.claude/agents/`, `.cursor/rules/`, `.kiro/steering/` from canonical `agents/personas/*.md`; run via lefthook pre-commit

#### Tier 2 — Security & supply chain
- [ ] Add `deny.toml` + `cargo-deny check` step in CI (advisory + license + duplicate dep gates)
- [ ] Add `.github/dependabot.yml` (or Renovate) for cargo + GitHub Actions ecosystems
- [ ] Add `SECURITY.md` (disclosure policy, contact, scope, response SLA)
- [ ] Add `.github/CODEOWNERS` — specifically protect `agents/personas/`, `scripts/`, `agents/schemas/`, `release.yml`
- [ ] Add SLSA build provenance via `actions/attest-build-provenance` in `release.yml`
- [ ] Add Sigstore `cosign` signing of release tarballs (keyless OIDC from workflow)
- [ ] Add SBOM generation (`cargo-cyclonedx` or `anchore/syft`) as a release asset
- [ ] Pin `install.sh` to a versioned release via `VERSION` env var (already in backlog under Component 11; consolidate)

#### Tier 3 — Inner test/feedback loop
- [ ] Adopt `cargo-nextest` for parallel test runs (faster local + CI feedback)
- [ ] Add `cargo-llvm-cov` coverage with CI threshold; publish to Codecov or commit baseline
- [ ] Add `proptest` property tests for FTS sanitizer, RRF determinism, actor-scoping invariants, NaN/Infinity validation
- [ ] Add `cargo-fuzz` target for the FTS query sanitizer (run nightly via scheduled workflow)
- [ ] Add `cargo-mutants` on critical modules (`db.rs`, `search.rs`, `memories.rs`)
- [ ] Add `insta` snapshot tests for MCP tool response envelopes (regression-detect schema drift)
- [ ] Add `cargo-machete` and `cargo-udeps` checks in CI (fail on unused deps)
- [ ] Add `promptfoo.yaml` regression tests for each persona (assert on output schema + finding counts on fixtures)
- [ ] Add `inspect/codeipi.eval.py` — UK AISI's CodeIPI eval to test prompt-injection resilience of personas
- [ ] Add `inspect-ai` end-to-end agent eval that exercises `local-memory-mcp` through real agent loops in a Docker sandbox

#### Tier 4 — Gates, state, hand-offs
- [ ] Add `.github/workflows/agent-review.yml` — runs `scripts/code-review.sh` on every PR with `LLM_MODEL` from a workflow variable
- [ ] Introduce `components.toml` typed state file (phase, gate_status, blockers, finding-file refs); replaces TODO.md phase tracking
- [ ] Adopt conventional commits (`feat:`, `fix:`, `chore:`) + `git-cliff` for `CHANGELOG.md` generation
- [ ] Add tag-vs-`Cargo.toml` version consistency check in `release.yml` (already in Component 10 backlog; consolidate)
- [ ] Add `lefthook.yml` — cross-vendor git hooks (fmt, clippy, deny, gitleaks, nextest); replaces vendor-specific hook configs
- [ ] Add `.github/PULL_REQUEST_TEMPLATE.md` with "personas reviewed" checkbox + link to `agents/WORKFLOW.md`
- [ ] Add `.editorconfig` and `CONTRIBUTING.md` (point at WORKFLOW.md for the agentic flow)

#### Tier 5 — Observability, scale, ops polish
- [ ] Add OpenLLMetry / OpenInference instrumentation (OTLP export) for end-to-end traces covering agent harness + MCP server
- [ ] Add MCP rate-limiting + per-tool request-size caps (token-bucket via `governor`; bound blob size, batch length, traversal depth)
- [ ] Add cross-platform install.sh CI test job (Linux x86_64, Linux aarch64, macOS arm64, macOS x86_64) with `bats-core` + `shellcheck`
- [ ] Add reliability test suite for SQLite failure modes (disk-full via tmpfs `--size`, corrupted page, SIGKILL during write)
- [ ] Add `benches/` directory with `criterion` benchmarks (FTS query, vector KNN over N memories, RRF fusion, graph traversal at depth)

#### Tier 6 — Frontier (optional, longer-horizon experiments)
- [ ] Validator script: assert `src/<component>.rs` exposes exactly the design's declared API surface (executable spec compliance)
- [ ] LLM-as-judge aggregator using `inspect-ai` primitives, replacing the prose dedupe rules in `PERSONA_IMPROVEMENTS.md`
- [ ] Persona versioning + retroactive re-review (when `sec_review.md` v3→v4, query `components.toml` for components reviewed against v3 and re-run)
- [ ] Sandboxed coding sub-agent runs via `inspect-ai`'s Docker/K8s sandbox primitives
- [ ] Dogfood `local-memory-mcp` itself via `.mcp.json` (any vendor that supports MCP gets the agent's memory)
- [ ] Autonomous scheduled review loop — GitHub Actions cron runs `scripts/code-review.sh` against `main`, opens issue on new findings

### LLM Harness Discoverability

See `design/llm-discoverability.md` for full audit, findings (F1–F6),
and rationale. Goal: make it easier for LLM harnesses (Claude Code,
Cursor, Codex, etc.) to find and correctly use the server's 29 tools.

#### Tier 1 — In-place, no surface break (land first)
- [x] R1: Override `MemoryServer::get_info()` to set `server_info` (fix `"rmcp"` → `"local-memory-mcp"` identity, add title + description) and `instructions` block (actor_id concept, namespace convention, embedding contract, strategy vocabulary, intent→tool decision list)
- [x] R2: Add `#[schemars(description = ...)]` to every non-obvious param field — `actor_id`, `session_id`, `strategy`, `metadata` (clarify JSON-object-string + example), `namespace`/`namespace_prefix`, `embedding`/`new_embedding` (note caller-computed), all `*_id` UUID fields, `properties`
- [x] R5: Add `ToolAnnotations` to all tools — `readOnlyHint` on get/list/recall/traverse/stats, `destructiveHint` on all `delete_*`, `idempotentHint` on `create_namespace`/`switch_store`, friendly `title` on every tool
- [x] R6: Rewrite descriptions of similar-sibling tools with explicit "use this for X, not Y" discriminators — `memory.list` ↔ `memory.recall`, `memory.get_event` ↔ `memory.get_events` ↔ `memory.list_sessions`, `graph.get_neighbors` ↔ `graph.traverse`

#### Tier 2 — Surface changes, bundle for v0.2

AgentCore-aligned rename. Canonical mapping is in
`design/agentcore-parity.md`; rollout confirmed as a hard rename
(no aliases). Net: 12 tool renames + 4 namespace moves;
13 tools unchanged.

##### Tool renames
- [x] Rename `memory.add_event` → `memory.create_event` (AgentCore: `CreateEvent`)
- [x] Rename `memory.get_events` → `memory.list_events` (AgentCore: `ListEvents`)
- [x] Rename `memory.delete_expired` → `memory.delete_expired_events`
- [x] Rename `memory.store` → `memory.create_memory_record` (AgentCore: `CreateMemoryRecord`/`BatchCreateMemoryRecords`); resolves the `memory.store`/`memory.list_stores` noun-verb collision
- [x] Rename `memory.get` → `memory.get_memory_record` (AgentCore: `GetMemoryRecord`)
- [x] Rename `memory.list` → `memory.list_memory_records` (AgentCore: `ListMemoryRecords`)
- [x] Rename `memory.recall` → `memory.retrieve_memory_records` (AgentCore: `RetrieveMemoryRecords`); highest-leverage rename — surfaces "retrieve"/"search" intent
- [x] Rename `memory.consolidate` → `memory.update_memory_record` (closest AgentCore op: `BatchUpdateMemoryRecords`)
- [x] Rename `memory.delete` → `memory.delete_memory_record` (AgentCore: `DeleteMemoryRecord`)
- [x] Rename `memory.checkpoint` → `memory.create_checkpoint`
- [x] Rename `memory.branch` → `memory.create_branch`
- [x] Rename `graph.add_edge` → `graph.create_edge`
- [x] Rename `graph.stats` → `graph.get_stats`

##### Namespace moves
- [x] Move `memory.switch_store` → `store.switch`
- [x] Move `memory.current_store` → `store.current`
- [x] Move `memory.list_stores` → `store.list`
- [x] Move `memory.delete_store` → `store.delete`

##### Field renames
- [x] Rename `memory_id` → `memory_record_id` on `GetMemoryParams`, `ConsolidateParams`, `DeleteMemoryParams`; resolves AgentCore semantic collision (their `memoryId` is the resource, not a record)
- [x] Rename `from_memory_id` / `to_memory_id` / `start_memory_id` → `from_memory_record_id` / `to_memory_record_id` / `start_memory_record_id` on graph params
- [x] Rename `query` → `search_query` on `memory.retrieve_memory_records` (matches AgentCore `searchQuery`)
- [x] Rename `limit` → `top_k` on `memory.retrieve_memory_records` (matches AgentCore `topK`); other tools keep `limit`

##### Description rewrites (apply to all tools using the parity-doc style guide)
- [x] Apply the "Use this when X; use sibling Y instead for Z. … (AgentCore equivalent: Op)" template from `design/agentcore-parity.md` §"Description style guide" to every tool description; worked examples already drafted for `memory.retrieve_memory_records`, `memory.create_memory_record`, `memory.list_events`, `memory.update_memory_record`, `memory.create_event`, `store.switch`, `graph.create_edge`

##### Source-side cleanup that ships in the same PR
- [x] Update `README.md` tool tables with v0.2 names + "Upgrading from v0.1" section
- [x] Update tool-name references in `design/DESIGN.md`, `design/mcp-server.md`, `design/memory-tools.md`, `design/event-tools.md`, `design/session-tools.md`, `design/namespace-tools.md`, `design/knowledge-graph.md`, `design/search.md`
- [x] Update tool-name references in `tests/integration.rs` and `tests/e2e.rs`
- [x] Bump `version` in `Cargo.toml` to `0.2.0`
- [x] Create `CHANGELOG.md` with v0.2.0 breaking-changes table

##### Code review — completed 2026-04-29
- [x] RelReview High: `cargo fmt --check` failures — ran `cargo fmt`, CI now passes
- [x] InteropReview High: `UpdateMemoryRecordParams.action` missing schemars description — added
- [x] InteropReview High: `SwitchStoreParams.name` + `DeleteStoreParams.name` missing schemars — added
- [x] SecReview/RelReview Medium: CHANGELOG missing `query` → `search_query` field rename row — added, with supplemental grep note

##### Other Tier-2 items (independent of the rename)
- [ ] R7: Change `metadata` and graph `properties` from `Option<String>` to `Option<serde_json::Value>` so the JSON Schema reflects the actual object shape
- [ ] R8: Optional — replace dots with underscores in tool names (`memory_create_event`, `graph_traverse`) for cross-host portability; if keeping dots, document host-compatibility requirement in README

### From LLM discoverability Tier 2 code review (Medium/Low)
- [ ] CHANGELOG migration grep: scoped note for `"query"` and `"limit"` only applies to `memory.retrieve_memory_records`; other tools keep `limit` (SecReview Medium)
- [ ] InteropReview: `graph.traverse` lacks a noun — `namespace.verb` rather than `namespace.verb_noun` convention; acceptable but undocumented (InteropReview Medium)
- [ ] InteropReview: `graph.get_stats` annotation title is "Graph stats" not "Get graph stats" — minor inconsistency (InteropReview Low)
- [ ] InteropReview: `memory.list_sessions` claims `AgentCore equivalent: ListSessions` but this op is not documented in AgentCore Memory API; change to `(Local-only extension: lists sessions for actor.)` (InteropReview Low)
- [ ] `CreateNamespaceToolParams.name` still missing schemars description (MaintReview note; Tier 1 backlog A2/A3)
- [ ] MaintReview: internal domain struct fields `from_memory_id`/`to_memory_id` in `GraphInsertEdgeParams` diverge from MCP wire names — intentional adapter, but could confuse future editors (MaintReview Low)

### From LLM discoverability Tier 1 code review (Medium/Low)
- [ ] Replace "email hash" example in SERVER_INSTRUCTIONS with "UUID or opaque per-user identifier" (SecReview Low)
- [x] Add schemars description to `name` field on `SwitchStoreParams` and `DeleteStoreParams` — fixed in Tier 2 code review (Interop High); `CreateNamespaceToolParams.name` still needs it
- [ ] Add schemars description to `BranchToolParams.parent_branch_id` and `GetEventsToolParams.branch_filter` (valid values: "all", "main", branch UUID) (ArchReview Medium A4)
- [ ] Add schemars description to `direction` field on `GetNeighborsParams` and `TraverseParams` (doc comment only, not in JSON Schema) (ArchReview Low A5)
- [ ] Add sentence to `memory.switch_store` and `memory.delete_store` descriptions: "This tool does not require actor_id — it operates on the store globally." (ArchReview Low A6)
- [x] Add schemars description to `UpdateMemoryRecordParams.action` field explaining 'update' vs 'invalidate' semantics — fixed in Tier 2 code review (Interop High)
- [ ] Add `// keep in sync with crate::db::EMBEDDING_DIM` comment next to "384 dims" in SERVER_INSTRUCTIONS and embedding field descriptions (RelReview Low; MaintReview Low M3; InteropReview Low F1)
- [ ] Add "Deletes across all actors — not scoped to a specific actor_id." to `memory.delete_expired` description (InteropReview Low F3)
- [ ] Add `// NOTE: actor_id description is repeated across all param structs — grep for the exact string to find all occurrences when updating.` comment before first occurrence (MaintReview Medium M1)
- [ ] Add `// TODO(v0.2): update tool names in SERVER_INSTRUCTIONS after Tier 2 rename lands` comment (MaintReview Low M5)
- [ ] Remove duplicate `/// JSON object string for edge properties` doc comment on `UpdateEdgeToolParams.properties` (MaintReview Low M4)

### Future features
- [ ] Local embedding model (ort + all-MiniLM-L6-v2)
- [ ] Automatic extraction (on-device LLM)
- [ ] Graph relationships between memories
- [ ] Import/export compatible with AgentCore Memory format
- [ ] Encryption at rest (sqlcipher)
- [ ] Web UI for browsing memories
