# Architecture Decision Records

---

## ADR-001: Storage engine — SQLite over SurrealDB

**Date**: 2026-04-26
**Status**: Accepted

**Context**: Need an embedded database for local agent memory. Must support event storage, full-text search, and vector similarity search. The predecessor project (kiro-graph) used SurrealDB embedded with RocksDB.

**Alternatives considered**:
1. **SurrealDB embedded** — Multi-model (graph + document + FTS + vector). Native graph edges. But: BSL 1.1 license restricts offering as a Database Service, 30-50MB binary overhead, minutes-long compile times, RocksDB directory complexity.
2. **ArangoDB** — Apache 2.0, multi-model. But: no first-party Rust driver, requires separate server process.
3. **CozoDB** — MPL 2.0, Rust-native, graph + vector. But: Datalog query language (steep learning curve), development slowing since Dec 2023.
4. **SQLite + FTS5 + sqlite-vec** — Public domain, single-file, battle-tested, ~2MB binary overhead, seconds to compile, ACID transactions.

**Decision**: SQLite. Agent memory access patterns (store events, search by text/vector, retrieve by session) don't require native graph traversal. SQLite covers relational + document (JSON) + FTS + vector with extensions. The simplicity, licensing, and maturity advantages are decisive.

**Consequences**: No native graph edges — relationships use an edges table + recursive CTEs if needed. No arrow traversal syntax. Acceptable tradeoff for agent memory.

---

## ADR-002: Language — Rust

**Date**: 2026-04-26
**Status**: Accepted (carried from kiro-graph)

**Decision**: Rust. Single compiled binary, no runtime deps, `rmcp` is the official MCP SDK, `rusqlite` is mature.

---

## ADR-003: Multi-store — Separate SQLite files

**Date**: 2026-04-26
**Status**: Accepted

**Context**: User needs isolated memory contexts. Only one store is used per session.

**Decision**: Each memory store is a separate `.db` file under `~/.local-memory-mcp/`. Full isolation, portable, independently deletable/backupable.

---

## ADR-004: MCP transport — stdio

**Date**: 2026-04-26
**Status**: Accepted (carried from kiro-graph)

**Decision**: stdio. Kiro's native transport. Binary launched on demand, communicates via stdin/stdout JSON-RPC. All logging to stderr.

---

## ADR-005: Embeddings provided by caller

**Date**: 2026-04-26
**Status**: Accepted

**Context**: AgentCore Memory uses managed Bedrock models to generate embeddings. Locally, we need a strategy for vector search.

**Alternatives considered**:
1. **Bundle an ONNX model** — Self-contained but adds ~50MB to binary, requires `ort` crate, complex cross-compilation.
2. **Caller provides embeddings** — The agent (Kiro) generates embeddings and passes them as vectors. Server stays simple.
3. **No vector search in MVP** — FTS5 only. Add vectors later.

**Decision**: Caller provides embeddings. The server stores and indexes vectors but doesn't generate them. This keeps the binary small and dependency-free. A future ADR can revisit bundling a local model.

---

## ADR-006: Memory extraction is agent-driven

**Date**: 2026-04-26
**Status**: Accepted

**Context**: AgentCore Memory automatically extracts insights from events using LLM-based strategies. Locally, we don't have a managed LLM.

**Decision**: The agent (Kiro) performs extraction and calls `memory.store` with the insight text. The server is a storage layer, not an intelligence layer. This keeps the server simple and lets the agent use whatever model it has access to.

---

## Pivots

### Pivot 1: kiro-graph → local-memory-mcp (2026-04-26)

**What changed**: Entire project scope and storage engine.
**Why**: The original kiro-graph project was a knowledge graph tool using SurrealDB. After evaluating the SurrealDB BSL 1.1 license, researching alternatives, and clarifying the actual need (agent memory, not a general graph database), we pivoted to a focused agent memory server using SQLite.
**Rewrite scope**: Full design rewrite. Carried forward: development workflow, review personas, tracking files, lessons learned, Rust + rmcp + stdio decisions.

---

## ADR-007: Db trait as API contract boundary

**Date**: 2026-04-26
**Status**: Accepted

**Context**: Components 2-7 (events, memories, graph, search, sessions, namespaces) all need database access. The original design exposed `&Connection` directly via `StoreManager::conn()`. This means every component writes raw SQL, creating conflicts when multiple coding agents work in parallel — they'd step on each other's query patterns, error handling, and table access assumptions.

**Alternatives considered**:
1. **Raw `&Connection`** — Simple, but no contract boundary. Parallel agents conflict on SQL. Schema changes require updating every module.
2. **Repository pattern per component** — Each module has its own `EventRepo`, `MemoryRepo`, etc. Clean separation but duplicates connection management and makes cross-cutting queries (e.g., search across events + memories) awkward.
3. **Single `Db` trait** — One trait with all data operations. Implemented for `Connection`. Components call trait methods. All SQL in one place.

**Decision**: Single `Db` trait in `db.rs`. `StoreManager::db()` returns `&dyn Db`. The trait starts with store management methods in Component 1 and grows as each component adds its method signatures during its design phase. All SQL lives in `impl Db for Connection`.

**Consequences**: Slightly more upfront design work per component (must define trait method signatures before coding). But parallel agents can work independently against the trait contract. Schema changes are single-file. No SQL scattered across modules.
