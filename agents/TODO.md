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

## In Progress

- [ ] Component 1: Core DB layer — **Phase 1 design pending** (`design/core-db-layer.md`)

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

### Future features
- [ ] Local embedding model (ort + all-MiniLM-L6-v2)
- [ ] Automatic extraction (on-device LLM)
- [ ] Graph relationships between memories
- [ ] Import/export compatible with AgentCore Memory format
- [ ] Encryption at rest (sqlcipher)
- [ ] Web UI for browsing memories
