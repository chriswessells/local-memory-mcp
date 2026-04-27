# Persona Improvement Recommendations

Patterns observed across Components 1–4 (8 design reviews, 4 code reviews, 40 persona invocations) that would make the review personas more effective.

---

## All Personas

**Deduplicate across reviewers.** The same finding was frequently raised by 3–4 personas independently (e.g., overfetch constant duplication flagged by Security, Architecture, Maintainability, and Interop). Add an instruction: "Focus on findings within your specialty. If an issue is primarily about security, the security reviewer owns it — don't repeat it from an architecture angle unless you have a distinct architectural concern beyond the security one."

**Distinguish new issues from pre-existing ones.** In Component 4 code review, the Interop reviewer flagged the `unsafe transmute` in `ensure_sqlite_vec` as High — but this was pre-existing code from Component 1, already documented in ADR and code comments. Add an instruction: "If a finding exists in code that was NOT modified by this component, note it as 'pre-existing' and lower its effective severity. Focus review effort on new/changed code."

**Calibrate severity consistently.** The same duplicated-constants issue was rated High by Maintainability but Medium by Security and Low by Interop. Add severity calibration guidance:
- **Critical**: Data loss, security breach, or crash in production
- **High**: Correctness bug, silent wrong results, or will block future components
- **Medium**: Maintenance burden, inconsistency, or defense-in-depth gap
- **Low**: Style, documentation, or theoretical concern

---

## Security Reviewer (`sec_review.md`)

**Add "validate floating-point inputs" to the checklist.** The NaN/infinity embedding finding was the most actionable security item in Component 4, but it wasn't in the persona's focus areas. Add: "Numeric input validation (NaN, infinity, overflow, negative values where unsigned expected)."

**Add "trust boundary at trait/interface level" to the checklist.** The `search_fts` accepting unsanitized strings via the public Db trait was a real concern. Add: "Identify trust boundaries at public API/trait interfaces. Can callers bypass validation by calling lower-level methods directly?"

**Reduce false positives on resource exhaustion.** The reviewer flagged hybrid search memory usage (2000 Memory objects) as a concern, but `MAX_PAGE_LIMIT` already caps this. Add: "Before flagging resource exhaustion, check whether existing caps/limits already bound the concern. If they do, note it as 'bounded by X' rather than raising it as a finding."

---

## Architecture Reviewer (`arch_review.md`)

**Flag param struct consistency proactively.** The 6-parameter trait methods were caught in design review, but the pattern was already established by `list_memories` (which uses a struct). Add: "When reviewing new trait methods, compare parameter style against existing methods in the same trait. Flag inconsistencies."

**Add "score/metric semantics across abstraction boundaries" to the checklist.** The `Vec<(Memory, f64)>` return type carrying different semantics (BM25 vs L2 distance) was the most architecturally significant finding. Add: "When a return type carries a numeric value, verify the semantic meaning is documented or enforced at the type level. Raw `f64` crossing an abstraction boundary is a code smell."

---

## Maintainability Reviewer (`maint_review.md`)

**Check for dead code explicitly.** `VECTOR_OVERFETCH_FACTOR` and `MAX_K_OVERFETCH` in `search.rs` were defined but never used — the actual logic was in `db.rs` with duplicated local constants. Add: "Grep for constants and functions defined in the new code. Verify each is actually referenced. Flag dead code."

**Flag magic column indices.** The `row.get::<_, f64>(11)` pattern was flagged but only as Medium. In a codebase where `row_to_memory` uses indices 0–10, an implicit dependency on column 11 is fragile. Add: "When reviewing SQL result mapping, check for positional column access that depends on SELECT order. Flag any index that isn't tied to a named constant or comment."

**Require adversarial tests for sanitization functions.** The sanitizer had good happy-path tests but no adversarial injection tests. Add: "For any input sanitization function, require at least one test with a known attack payload (e.g., FTS5 operators, SQL injection strings, path traversal sequences)."

---

## Reliability Reviewer (`rel_review.md`)

**Distinguish "known limitation" from "bug".** The vector post-filter starvation was flagged in both design review and code review as a finding, even though the design explicitly documented it as a known limitation with rationale. Add: "If the design document explicitly acknowledges a limitation and provides rationale for accepting it, do not re-raise it as a finding. Instead, verify the documentation is accurate and the mitigation (if any) is implemented correctly."

**Add "silent semantic changes" to the checklist.** The hybrid→vector-only fallback changing score semantics was a subtle reliability concern. Add: "When a function has multiple code paths that produce the same output type, verify the output semantics are consistent across all paths. Flag cases where the same field means different things depending on which path executed."

---

## Interoperability Reviewer (`interop_review.md`)

**Don't flag pre-existing cross-cutting concerns.** The `PRAGMA locking_mode = EXCLUSIVE` and `OnceLock` findings were re-raised in Component 4 despite being Component 1 concerns. Add: "Only flag cross-platform/integration issues in code that was added or modified by this component. Pre-existing concerns should have been caught in their original component's review."

**Add "module wiring" to the checklist.** The `main.rs` missing `mod search;` was a real build failure. Add: "For new modules, verify the module is declared in both `lib.rs` and `main.rs` (for binary+library crate layouts). Check that the implementation plan includes this step early, not as a final step."

**Reduce speculative platform findings.** The "sqlite-vec KNN blob binding may fail on Windows with certain MSVC configurations" finding (rated Critical in round 1) was speculative — no evidence of actual failure. Add: "Platform-specific findings must cite evidence: a known bug report, a documented behavioral difference, or a test failure. Speculative 'may fail on platform X' findings should be Low severity with a recommendation to add a CI test, not Critical."
