# Development Workflow

This document defines the process for designing, reviewing, building, and shipping every component of local-memory-mcp.

---

## Principles

- Every component goes through **Design → Design Review → Code → Code Review** before it is done.
- All **Critical** and **High** findings must be resolved before advancing to the next phase.
- Work is tracked in `agents/TODO.md`. Decisions are recorded in `agents/ADR.md`. Mistakes are captured in `agents/LESSONS_LEARNED.md`. Time is logged in `agents/TIME_LOG.md`.
- These files are living documents — update them as you work, not after.

---

## Model Assignment

| Phase | Model | Rationale |
|-------|-------|-----------|
| Design (detailed design, implementation plan, DAG, sub-agent instructions) | **opus-4.6** | Strongest reasoning for architecture, edge cases, and instruction clarity |
| Design Review (5 personas) | **opus-4.6** | Needs deep analysis to find Critical/High issues |
| Coding (implementation via sub-agents) | **sonnet-4.6** | Fast, accurate code generation from clear instructions |
| Code Review (5 personas) | **opus-4.6** | Needs deep analysis to find implementation bugs |

The orchestrating agent (main chat) uses whichever model is active. Sub-agents spawned for coding tasks should use `sonnet-4.6`. Sub-agents spawned for review tasks should use `opus-4.6`.

---

## Phases

### Phase 1: Design *(opus-4.6)*

For each component, produce:

1. **Detailed design** — data flow, error handling, edge cases, schema, API surface
2. **Implementation plan** — ordered list of tasks with clear acceptance criteria
3. **DAG** — dependency graph showing which tasks can run in parallel vs. sequentially
4. **Sub-agent instructions** — step-by-step build instructions precise enough for a sub-agent to execute without ambiguity

All design artifacts go in the `design/` directory.

### Phase 2: Design Review *(opus-4.6)*

Run all five review personas against the design artifacts:

| Persona | File | Focus |
|---------|------|-------|
| SecReview | `agents/sec_review.md` | Security, injection, supply chain, least privilege |
| ArchReview | `agents/arch_review.md` | Architecture, API design, extensibility, tech choices |
| MaintReview | `agents/maint_review.md` | Maintainability, test strategy, deps, code org |
| RelReview | `agents/rel_review.md` | Reliability, failure modes, durability, recovery |
| InteropReview | `agents/interop_review.md` | Cross-platform, MCP compliance, installers, encoding |

Each persona produces findings with severity: **Critical | High | Medium | Low**.

**Gate**: Resolve all **Critical** and **High** findings. Log **Medium** and **Low** items in `TODO.md` backlog.

**Re-review rule**: After resolving Critical/High findings, assess whether the changes were substantial. **If changes were substantial, re-run all five personas on the revised design.** The gate is: reviewers approve the design that will actually be built.

**What counts as substantial**: Any change to a public API signature, any new module or struct, any change to the concurrency or synchronization model, any new dependency, any change to the data model or schema, any change to error handling strategy. When in doubt, re-review.

### Phase 3: Coding *(sonnet-4.6)*

**Pre-coding checklist** (all must be true before writing any code):
- [ ] Design doc exists in `design/` with: detailed design, implementation plan, DAG, sub-agent instructions
- [ ] All five review personas have reviewed the **final** version of the design
- [ ] Zero open Critical or High findings
- [ ] Medium/Low findings logged in `TODO.md` backlog
- [ ] If substantial changes were made after initial review, re-review was completed

Implement the approved design:

1. Follow the DAG and instructions from Phase 1
2. Write tests alongside code
3. Build must pass: `cargo check`, `cargo test`, `cargo clippy -- -D warnings`
4. Log time spent in `agents/TIME_LOG.md`
5. Record any deviations from the design in `agents/ADR.md`

### Phase 4: Code Review *(opus-4.6)*

Run all five review personas against the implementation.

**Gate**: Resolve all **Critical** and **High** findings before merging to `main`.

### Phase 5: Merge

Once all gates pass:
1. Commit to `main` with a descriptive message
2. Update `agents/TODO.md`
3. Update `agents/LESSONS_LEARNED.md` if anything surprised you
4. Log final time in `agents/TIME_LOG.md`

---

## Components

| # | Component | Scope |
|---|-----------|-------|
| 1 | Core DB layer | `db.rs`, `store.rs` — SQLite init, schema, store switching |
| 2 | Event tools | `events.rs`, `tools.rs` — add, get, expire events |
| 3 | Memory tools | `memories.rs`, `tools.rs` — store, recall, consolidate, list, delete |
| 4 | Search | `search.rs` — FTS5 + vector search integration |
| 5 | Session tools | `tools.rs` — checkpoints, branches |
| 6 | Store management tools | `tools.rs` — switch, list, delete stores |
| 7 | Namespace tools | `tools.rs` — create, list, delete namespaces |
| 8 | MCP server | `main.rs` — server init, stdio transport, shutdown |
| 9 | CI/CD | `.github/workflows/` — ci.yml, release.yml |
| 10 | Installers | `install.sh`, `install.ps1` |

---

## Tracking Files

| File | Purpose |
|------|---------|
| `TODO.md` | All work: completed, in-progress, planned. Single source of truth. |
| `ADR.md` | Architecture Decision Records. Every significant choice and pivot. |
| `LESSONS_LEARNED.md` | What went wrong, what surprised us, what we'd do differently. |
| `TIME_LOG.md` | Time spent on each task. |
