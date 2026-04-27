# local-memory-mcp — Session Start

Read this file at the beginning of every session to orient yourself.

## What is this project?

A local agent memory MCP server for Kiro. Compiled Rust binary, SQLite embedded, MCP server over stdio. Inspired by Amazon Bedrock AgentCore Memory but runs entirely locally with no cloud dependencies. See `design/DESIGN.md` for the full architecture.

## Directory layout

```
local-memory-mcp/
├── Start_session.md          ← You are here
├── design/                   # All designs, plans, and instructions
│   ├── DESIGN.md             # Main design document
│   ├── core-db-layer.md      # Detailed design for db.rs + store.rs
│   ├── event-tools.md        # Detailed design for events.rs
│   ├── memory-tools.md       # Detailed design for memories.rs
│   └── search.md             # Detailed design for search.rs
├── agents/                   # Review personas, workflow, and tracking
│   ├── WORKFLOW.md            # Development process: phases, gates, review execution
│   ├── sec_review.md          # Security reviewer persona
│   ├── arch_review.md         # Architecture reviewer persona
│   ├── maint_review.md        # Maintainability reviewer persona
│   ├── rel_review.md          # Reliability reviewer persona
│   ├── interop_review.md      # Interoperability reviewer persona
│   ├── TODO.md                # Work tracking: completed, in-progress, planned
│   ├── ADR.md                 # Architecture Decision Records and pivots
│   ├── LESSONS_LEARNED.md     # Retrospective notes
│   └── TIME_LOG.md            # Time spent per task
└── src/                       # Rust source code
```

## Before doing any work

1. Read `agents/TODO.md` to see what's done and what's next
2. Read `agents/WORKFLOW.md` to understand the phased process
3. Read `design/DESIGN.md` for architecture and data model context
4. Check `agents/ADR.md` for past decisions — don't re-litigate settled choices
5. Log your time in `agents/TIME_LOG.md` as you work

## Key rules

- Every component goes through **Design → Design Review → Code → Code Review**
- All **Critical** and **High** review findings must be resolved before advancing
- Designs go in `design/`, one file per component
- Review personas are in `agents/*.md` — use them for every review
- Update `agents/TODO.md`, `agents/LESSONS_LEARNED.md`, and `agents/TIME_LOG.md` as you go
- Record any new architectural decisions in `agents/ADR.md`
