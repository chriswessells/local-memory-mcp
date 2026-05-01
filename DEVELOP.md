# Development Guide

## Prerequisites

- Rust toolchain (stable) — install via [rustup](https://rustup.rs/)
- No other dependencies — SQLite and sqlite-vec are compiled from source via the `bundled` feature flag

## Building

```bash
cargo build           # debug build
cargo build --release # release build (~5-10MB binary)
```

## Testing

```bash
cargo test                      # run all tests
cargo clippy -- -D warnings     # lint with zero-warnings policy
cargo fmt --check               # verify formatting
```

153 tests across unit, integration, and E2E suites. Tests use `tempfile` for isolated SQLite databases — no fixtures or external services required.

---

## Contributing with a Coding Agent

This project is designed to be developed collaboratively with a coding agent (Claude Code, Cursor, Kiro, Codex, or similar). The workflow below is how every component has been built. Following it produces consistent, high-quality results and keeps the tracking files accurate for the next session.

### Step 1 — Orient the agent

Start every session by telling the agent to read `Start_session.md`. This file is the single source of truth for what the project is, where everything lives, and what the rules are. The agent will also read `agents/TODO.md` to find out what's done and what's next.

> **Example prompt:**
> *"Read Start_session.md and then tell me what's next on the TODO list."*

The agent will read the session file, orient itself, and summarize the current state. From there you can steer it toward the work you want to do.

### Step 2 — Describe what you want

Once the agent is oriented, describe the change or feature you have in mind in plain language. You don't need to specify implementation details — that's what the design phase is for. Focus on what problem you're solving or what behavior you want.

> **Example prompts:**
> - *"I want to add a tool that lets callers batch-delete expired events more efficiently."*
> - *"The metadata field should accept a real JSON object instead of a stringified JSON string."*
> - *"We need integration tests for the session and namespace tools."*

Have a back-and-forth conversation with the agent to clarify scope, constraints, and any relevant context before design begins. This conversation is cheap — it's far less expensive than discovering a misunderstood requirement after code is written.

### Step 3 — Design

Ask the agent to write a design document in `design/`. A good design doc for this project includes:

- **Data flow** — how data moves through the layers (MCP tool → business logic → `Db` trait → SQL)
- **Error handling** — every failure mode and what error variant it maps to
- **Schema changes** — any new tables, columns, or indexes with the exact DDL
- **API surface** — structs, method signatures, and the `Db` trait extension
- **Implementation plan** — an ordered list of tasks with acceptance criteria
- **Dependency graph** — which tasks can run in parallel and which must be sequential
- **Sub-agent instructions** — precise enough that an agent could execute them without asking follow-up questions

> **Example prompt:**
> *"Write a design doc for this change in design/. Follow the structure of existing design docs like design/memory-tools.md."*

### Step 4 — Design review

Once the design is written, ask the agent to run it through all five review personas. The personas live in `agents/` and each one focuses on a different dimension of quality.

| Persona | File | Focus |
|---------|------|-------|
| Security | `agents/sec_review.md` | Injection, privilege, supply chain, data leakage |
| Architecture | `agents/arch_review.md` | API design, extensibility, tech choices |
| Maintainability | `agents/maint_review.md` | Test strategy, dependencies, code organization |
| Reliability | `agents/rel_review.md` | Failure modes, durability, recovery paths |
| Interoperability | `agents/interop_review.md` | MCP compliance, cross-platform, encoding |

Each persona produces findings rated **Critical**, **High**, **Medium**, or **Low**.

> **Example prompt:**
> *"Run all five review personas against the design doc you just wrote."*

### Step 5 — Fix Critical and High findings

All **Critical** and **High** findings must be resolved before moving to code. Work through each one with the agent — some will require revising the design doc, others might require a new architectural decision. Log any new architectural decisions in `agents/ADR.md`.

Medium and Low findings go into the `agents/TODO.md` backlog. They are not blockers.

If the changes needed to resolve Critical or High findings were substantial — meaning they touched a public API signature, a new module or struct, the concurrency model, a new dependency, the data model, or error handling strategy — **re-run all five personas on the revised design** before proceeding. The gate is that reviewers approve the design that will actually be built, not a previous version of it.

> **Example prompt:**
> *"We resolved the High findings. The changes touched the Db trait signature and added a new struct. Please re-run the design review."*

### Step 6 — Code

With an approved design in hand, ask the agent to implement it. The agent should follow the design's implementation plan and dependency graph, write tests alongside the code, and verify the build stays clean throughout.

Every implementation must satisfy three gates before moving on:

```bash
cargo check                   # no compile errors
cargo test                    # all tests pass
cargo clippy -- -D warnings   # no lints
```

> **Example prompt:**
> *"Implement the approved design. Follow the tasks in order, write tests alongside the code, and verify cargo check, cargo test, and cargo clippy all pass before we move to review."*

### Step 7 — Code review

Once the implementation is working and tests pass, ask the agent to run all five personas against the code. The same personas used in design review now read the actual implementation looking for bugs, security issues, and maintainability problems that weren't visible at design time.

> **Example prompt:**
> *"Run all five review personas against the implementation."*

### Step 8 — Fix Critical and High findings

Same rule as design review — all **Critical** and **High** findings must be resolved before merging. Work through each one with the agent, re-run tests and clippy after fixes, and log Medium and Low findings to `agents/TODO.md`.

### Step 9 — Update the tracking files

Before ending the session, make sure the tracking files reflect what was done. This is what allows the next session — with you or anyone else — to pick up exactly where you left off.

- **`agents/TODO.md`** — mark completed tasks, move items from In Progress to Completed, add any new backlog items surfaced during review
- **`CHANGELOG.md`** — record any user-facing changes (new tools, renamed parameters, breaking changes, bug fixes)
- **`agents/TIME_LOG.md`** — log how long each phase took

> **Example prompt:**
> *"Update TODO.md, CHANGELOG.md, and TIME_LOG.md to reflect the work we just completed."*

---

## Project Structure

```
local-memory-mcp/
├── Start_session.md              # Agent session orientation — read this first
├── README.md
├── DEVELOP.md                    # This file
├── CHANGELOG.md                  # User-facing change history
├── Cargo.toml
├── Cargo.lock                    # Committed — pinned dependencies
│
├── design/                       # Architecture and component designs
│   ├── DESIGN.md                 # Main design document (data model, MCP tools, principles)
│   ├── agentcore-parity.md       # AgentCore Memory alignment and rename mapping
│   ├── llm-discoverability.md    # LLM harness discoverability audit and recommendations
│   ├── core-db-layer.md          # Component 1: db.rs + store.rs
│   ├── event-tools.md            # Component 2: events.rs
│   ├── memory-tools.md           # Component 3: memories.rs
│   ├── search.md                 # Component 4: search.rs
│   ├── knowledge-graph.md        # Component 5: graph.rs
│   ├── session-tools.md          # Component 6: sessions.rs
│   ├── namespace-tools.md        # Component 8: namespaces.rs
│   ├── mcp-server.md             # Component 9: tools.rs + main.rs
│   ├── ci-cd.md                  # Component 10: GitHub Actions workflows
│   ├── installers.md             # Component 11: install.sh
│   ├── integration-tests.md      # Component 12: tests/
│   └── graceful-shutdown.md      # Bug fix: SIGTERM + WAL checkpoint
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
├── tests/
│   ├── common/mod.rs             # Shared test helpers (setup, parse_ok, parse_err)
│   ├── integration.rs            # Integration tests — full tool round-trips
│   └── e2e.rs                    # E2E tests — binary process over stdio MCP
│
└── src/
    ├── main.rs                   # Binary entry point, signal handling, graceful shutdown
    ├── lib.rs                    # Library crate (module declarations)
    ├── db.rs                     # SQLite connection, schema migration, Db trait + impl
    ├── store.rs                  # StoreManager (multi-store lifecycle)
    ├── error.rs                  # MemoryError enum
    ├── events.rs                 # Short-term memory: types, validation, business logic
    ├── memories.rs               # Long-term memory: types, validation, business logic
    ├── search.rs                 # FTS5 + vector search, RRF hybrid fusion, query sanitization
    ├── graph.rs                  # Knowledge graph: edges, traversal, neighbor queries
    ├── sessions.rs               # Session tools: checkpoints, branches
    ├── namespaces.rs             # Namespace registry: create, list, delete
    └── tools.rs                  # MCP tool handlers (MemoryServer, 29 tools)
```

## Design Principle: AgentCore Memory Compatibility

An agent with a system prompt should be able to use either AgentCore Memory or local-memory-mcp and not know the difference.

Same conceptual model, same tool semantics, same data lifecycle. The only transparent differences:

- **Extraction is explicit** — The agent calls `memory.create_memory_record` instead of extraction happening automatically
- **Embeddings are caller-provided** — The agent provides 384-dim vectors; the server stores and indexes them
- **Store management is additive** — `store.*` tools are a local-only extension
- **Knowledge graph is additive** — `graph.*` tools are a local-only extension

## Architecture

### System overview

```
┌─────────────┐     stdio (JSON-RPC)      ┌──────────────────────────┐
│   Kiro CLI   │ ◄──────────────────────► │  local-memory-mcp binary │
└─────────────┘                           │                          │
                                          │  rmcp (MCP SDK)          │
                                          │  Memory Engine           │
                                          │  SQLite + FTS5           │
                                          │  + sqlite-vec            │
                                          └──────────┬───────────────┘
                                                     │
                                                     ▼
                                          ~/.local-memory-mcp/
                                              default.db
                                              work.db
                                              ...
```

| Choice | Rationale |
|--------|-----------|
| Rust | Single compiled binary, no runtime deps |
| SQLite (rusqlite, bundled) | Embedded, single-file, ACID, public domain |
| FTS5 | BM25 ranking, prefix queries, built into SQLite |
| sqlite-vec | Embeddable vector similarity search |
| rmcp | Official Rust MCP SDK |
| stdio transport | Kiro's native MCP transport |

### Internal layers

All database operations go through the `Db` trait defined in `db.rs`. Downstream modules (`events.rs`, `memories.rs`, `search.rs`, etc.) accept `&dyn Db` and never write raw SQL — all SQL lives in the `impl Db for Connection` block. This keeps the API surface explicit and testable.

```
┌─────────────┐     ┌──────────────┐     ┌──────────────┐
│  tools.rs   │────►│  events.rs   │────►│  Db trait    │
│  (MCP layer)│     │  memories.rs │     │  (db.rs)     │
│  29 tools   │     │  search.rs   │     │              │
│             │     │  graph.rs    │     │  impl Db for │
│             │     │  sessions.rs │     │  Connection  │
│             │     │  namespaces.rs│    │              │
└─────────────┘     └──────────────┘     └──────────────┘
```

`StoreManager` in `store.rs` manages the SQLite connection lifecycle. It's wrapped in `Arc<Mutex<StoreManager>>` and accessed via `tokio::task::spawn_blocking` since `rusqlite::Connection` is `!Send`.

## Key Design Decisions

All architectural decisions are documented with full rationale in `agents/ADR.md`. The most consequential ones:

| Decision | Choice | Why |
|----------|--------|-----|
| Database | SQLite (not SurrealDB) | Public domain license, ~2MB binary vs ~50MB, no runtime daemon |
| Store isolation | One `.db` file per store | Full isolation, portable, independently deletable |
| Embeddings | Caller-provided | Keeps binary small, no bundled model, works with any embedding provider |
| Memory extraction | Agent-driven | Server is a storage layer, not an intelligence layer |
| SQL boundary | `Db` trait | All SQL centralized, safe for parallel agent development |

## Dependencies

Critical dependencies are pinned to exact versions to ensure reproducible builds:

| Crate | Version | Purpose |
|-------|---------|---------|
| `rusqlite` | =0.35.0 | SQLite bindings (bundled) |
| `sqlite-vec` | =0.1.7-alpha.10 | Vector similarity search extension |
| `rmcp` | =1.5.0 | Official Rust MCP SDK |
| `tokio` | 1 | Async runtime |
| `thiserror` | 2 | Error derive macros |
| `serde` / `serde_json` | 1 | Serialization |
| `schemars` | 0.8 | JSON Schema generation for MCP tool parameters |
| `uuid` | 1 | UUID v4 generation |
| `base64` | 0.22 | Blob encoding for MCP JSON transport |
| `dirs` | 6 | Platform-appropriate home directory |
| `tracing` / `tracing-subscriber` | 0.1 | Structured logging to stderr |

Dev: `tempfile = "3"` for test isolation.

## Releases

Binary releases are triggered by pushing a version tag — **not** by commits to `main`. CI runs tests on every push to `main`, but no binary is produced until a tag is pushed.

```bash
# To cut a release (maintainers only):
git tag v0.2.0.1
git push origin v0.2.0.1
```

The release workflow cross-compiles for four targets: `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `aarch64-apple-darwin`, and `x86_64-pc-windows-msvc`.

## How This Was Built

This project was developed entirely using a structured, agent-driven process over approximately **20 hours** of wall-clock time, spread across four days. The result is a production-ready Rust MCP server with 29 tools, 153 tests, a cross-platform CI/CD pipeline, and a one-command installer — work that would typically represent several weeks of traditional development effort.

### The development process

Every component followed the same four-phase workflow, with no exceptions:

**1. Design first.** Before writing a single line of code, the agent produced a detailed design document covering data flow, error handling strategy, schema changes, the full API surface, an implementation plan, and a dependency graph of parallel vs. sequential tasks. All design artifacts live in `design/` as version-controlled, reviewable documents.

**2. Multi-perspective design review.** Five specialized review personas — security, architecture, maintainability, reliability, and interoperability — each read the design and produced findings rated Critical, High, Medium, or Low. Every Critical and High finding was resolved before any code was written. Medium and Low items were logged to `agents/TODO.md` as backlog. If the fixes were substantial, the full review panel re-ran against the revised design. The gate was simple: reviewers had to approve the design that would actually be built, not a prior version of it.

**3. Code against the approved design.** The agent implemented the approved design following its own dependency graph, writing tests alongside the code. The build had to stay clean — `cargo check`, `cargo test`, and `cargo clippy -- -D warnings` — throughout.

**4. Code review.** The same five personas reviewed the implementation. All Critical and High findings were fixed before merging to `main`. Every component's architectural decisions were recorded in `agents/ADR.md`, and the retrospective notes in `agents/LESSONS_LEARNED.md` informed the design of the next component.

The tracking files — `agents/TODO.md`, `agents/TIME_LOG.md`, `agents/ADR.md` — meant that any session, with any agent or human, could pick up exactly where the previous one left off without re-deriving context.

### Why this works better than other approaches

**Compared to vibe coding** — Vibe coding is fast to start: describe what you want, accept what the agent produces, iterate until it roughly works. For small scripts and throwaway tools, that's fine. For a project with 12 interdependent components, concurrent read/write paths, multiple search modes, and a security boundary between actors, architecture has to be intentional. Vibe coding produces architecture by accident. Critical issues — actor isolation gaps, SQL injection vectors, data loss on concurrent writes — surface after they're baked in, when they're expensive to fix. This process catches them in the design phase, before they exist in code.

**Compared to developer-directed coding** — In developer-directed coding, the human writes the spec: detailed requirements, interface definitions, maybe pseudocode. The agent fills in the implementation. This is more disciplined than vibe coding, but the human is still the primary reasoning engine — responsible for identifying every failure mode, every edge case, every security implication. The agent is a fast typist. In this process, the agent does the design reasoning too. The developer steers direction, validates quality gates, and makes judgment calls — but the five review personas do the heavy lifting of finding what was missed. The human doesn't need to be an expert in every dimension simultaneously; the personas provide that specialization.

**Compared to tab-complete coding** — Inline completions (Copilot, Cursor) work at the line or function level. The model sees local context and predicts what comes next — excellent for boilerplate, common patterns, and filling in known shapes. But a completion model doesn't know whether the function it's suggesting fits the broader architecture, whether it introduces a new SQL injection surface, or whether it's consistent with the actor-isolation invariant established three files away. It has no design document to check against and no cross-component view. This process works at the component level, with explicit design artifacts that make cross-cutting concerns visible before any implementation begins.

The common thread is that structure creates leverage. A five-minute design review that catches a Critical security finding prevents hours of remediation later. A tracked architectural decision prevents the next session from re-litigating a settled question. Tests written against a reviewed design have a clear target to hit. None of this requires a human to do the review or write the design — but it does require that someone asks for it.
