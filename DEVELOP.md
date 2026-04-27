# Development Guide

## Prerequisites

- Rust toolchain (stable) — install via [rustup](https://rustup.rs/)
- No other dependencies — SQLite and sqlite-vec are compiled from source via `bundled` feature

## Building

```bash
cargo build           # debug build
cargo build --release # release build (~5-10MB binary)
```

## Testing

```bash
cargo test                        # run all tests
cargo test -- --test-threads=1    # serial execution (if tests share resources)
cargo clippy -- -D warnings       # lint with zero warnings policy
```

81 tests across 4 components. Tests use `tempfile` for isolated SQLite databases — no test fixtures or external services needed.

## Project Structure

```
local-memory-mcp/
├── Start_session.md              # AI agent session orientation
├── README.md
├── DEVELOP.md                    # This file
├── Cargo.toml
├── Cargo.lock                    # Committed — pinned dependencies
│
├── design/                       # Architecture and component designs
│   ├── DESIGN.md                 # Main design document (data model, MCP tools, principles)
│   ├── core-db-layer.md          # Component 1: db.rs + store.rs
│   ├── event-tools.md            # Component 2: events.rs
│   ├── memory-tools.md           # Component 3: memories.rs
│   └── search.md                 # Component 4: search.rs
│
├── agents/                       # Development process and tracking
│   ├── WORKFLOW.md               # Phased process: Design → Review → Code → Review → Merge
│   ├── TODO.md                   # Work tracking (completed, in-progress, planned, backlog)
│   ├── ADR.md                    # Architecture Decision Records
│   ├── LESSONS_LEARNED.md        # Retrospective notes
│   ├── TIME_LOG.md               # Time spent per task
│   ├── sec_review.md             # Security reviewer persona
│   ├── arch_review.md            # Architecture reviewer persona
│   ├── maint_review.md           # Maintainability reviewer persona
│   ├── rel_review.md             # Reliability reviewer persona
│   └── interop_review.md         # Interoperability reviewer persona
│
└── src/
    ├── main.rs                   # Binary entry point
    ├── lib.rs                    # Library crate (module declarations)
    ├── db.rs                     # SQLite connection, schema migration, Db trait + impl
    ├── store.rs                  # StoreManager (multi-store lifecycle)
    ├── error.rs                  # MemoryError enum
    ├── events.rs                 # Short-term memory types, validation, business logic
    ├── memories.rs               # Long-term memory types, validation, business logic
    └── search.rs                 # FTS5 + vector search, RRF hybrid, sanitization
```

## Architecture

All database operations go through the `Db` trait defined in `db.rs`. Downstream modules (`events.rs`, `memories.rs`, `search.rs`) accept `&dyn Db` and never write raw SQL. All SQL lives in `impl Db for Connection`.

```
┌─────────────┐     ┌──────────────┐     ┌─────────────┐
│  tools.rs    │────►│  events.rs   │────►│  Db trait    │
│  (MCP layer) │     │  memories.rs │     │  (db.rs)     │
│  Component 8 │     │  search.rs   │     │              │
│              │     │              │     │  impl Db for │
│              │     │              │     │  Connection   │
└─────────────┘     └──────────────┘     └─────────────┘
```

`StoreManager` in `store.rs` manages the SQLite connection lifecycle. It's wrapped in `Arc<std::sync::Mutex<StoreManager>>` and accessed via `tokio::task::spawn_blocking` since `rusqlite::Connection` is `!Send`.

## Development Workflow

Every component goes through: **Design → Design Review → Code → Code Review → Merge**.

1. **Design** — Write a detailed design doc in `design/` with data flow, error handling, implementation plan, DAG, and sub-agent instructions
2. **Design Review** — Run all 5 review personas (security, architecture, maintainability, reliability, interoperability). Resolve all Critical and High findings. Log Medium/Low to `TODO.md` backlog. Re-review if changes were substantial.
3. **Code** — Implement the approved design. Write tests alongside code. All three must pass: `cargo check`, `cargo test`, `cargo clippy -- -D warnings`
4. **Code Review** — Run all 5 review personas against the implementation. Resolve all Critical and High findings.
5. **Merge** — Commit to `main`. Update `TODO.md`, `LESSONS_LEARNED.md`, `TIME_LOG.md`.

See `agents/WORKFLOW.md` for the full process definition.

## Key Design Decisions

Documented in `agents/ADR.md`:

- **ADR-001**: SQLite over SurrealDB — license (public domain vs BSL 1.1), binary size (~2MB vs ~30-50MB), build time
- **ADR-003**: Separate SQLite files per store — full isolation, portable, independently deletable
- **ADR-005**: Embeddings provided by caller — keeps binary small, no bundled model
- **ADR-006**: Memory extraction is agent-driven — server is a storage layer, not an intelligence layer
- **ADR-007**: `Db` trait as API contract — all SQL centralized, parallel-safe development

## Dependencies

Critical dependencies are pinned to exact versions:

| Crate | Version | Purpose |
|-------|---------|---------|
| `rusqlite` | =0.35.0 | SQLite bindings (bundled) |
| `sqlite-vec` | =0.1.7-alpha.10 | Vector similarity search extension |
| `rmcp` | =1.5.0 | Official Rust MCP SDK |
| `tokio` | 1 | Async runtime |
| `thiserror` | 2 | Error derive macros |
| `serde` / `serde_json` | 1 | Serialization |
| `uuid` | 1 | UUID v4 generation |
| `dirs` | 6 | Platform-appropriate home directory |
| `tracing` | 0.1 | Structured logging |

Dev: `tempfile = "3"` for test isolation.

## Adding a New Component

1. Read `agents/TODO.md` for the next planned component
2. Read `design/DESIGN.md` for the data model and MCP tool surface
3. Read `agents/ADR.md` to avoid re-litigating settled decisions
4. Write a design doc in `design/` following the pattern of existing component designs
5. Follow the workflow in `agents/WORKFLOW.md`
6. Log time in `agents/TIME_LOG.md` as you work
