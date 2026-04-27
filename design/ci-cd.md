# Component 10: CI/CD — Detailed Design

## Problem

The project has 111 tests (101 unit + 8 integration + 2 E2E) and uses `cargo clippy -- -D warnings` as a quality gate, but nothing enforces these checks automatically. A broken commit on `main` won't be caught until someone manually runs `cargo test`.

## Solution

Two GitHub Actions workflows:

1. **`ci.yml`** — runs on every push and PR to `main`. Checks formatting, lints, builds, and tests across Linux and macOS.
2. **`release.yml`** — runs on version tags (`v*`). Builds release binaries for Linux (x86_64, aarch64) and macOS (aarch64), creates a GitHub Release with the artifacts.

---

## CI Workflow (`ci.yml`)

### Triggers

```yaml
on:
  push:
    branches: [main]
  pull_request:
    branches: [main]
```

### Permissions

Explicit least-privilege:

```yaml
permissions:
  contents: read
```

### Concurrency

Cancel superseded runs on the same branch to save Actions minutes:

```yaml
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true
```

### Matrix

| OS | Runner | Rationale |
|----|--------|-----------|
| Linux x86_64 | `ubuntu-latest` | Primary dev/deploy target |
| macOS aarch64 | `macos-latest` | Developer machines (Apple Silicon) |

Two platforms are sufficient. Windows is not a target.

### Steps

1. **Checkout** — `actions/checkout` (SHA-pinned)
2. **Install Rust toolchain** — `dtolnay/rust-toolchain@stable` with `components: rustfmt, clippy`
3. **Cache** — `Swatinem/rust-cache` (SHA-pinned)
4. **Format check** — `cargo fmt --check` (fail-fast, no point building if formatting is wrong)
5. **Clippy** — `cargo clippy --all-targets -- -D warnings`
6. **Test** — `cargo test` (includes unit, integration, and E2E MCP protocol tests)

### Timeout

`timeout-minutes: 15` on the job. The full suite completes in ~1 minute locally; 15 minutes is generous for cold CI runners.

### E2E tests in CI

`cargo test` runs all tests including the 2 E2E tests that spawn the binary and communicate over MCP JSON-RPC. These tests use 5-second per-operation timeouts which should be sufficient on CI runners. If flakiness appears, the mitigation is to split E2E into a separate step or increase timeouts in the test code.

### Why not separate jobs for fmt/clippy/test?

Separate jobs each pay the checkout + toolchain + cache restore cost (~30s). For a project this size, a single sequential job is faster end-to-end. The steps are ordered fail-fast: fmt (instant) → clippy (~30s) → test (~10s).

### Rust version

Use `stable` via `dtolnay/rust-toolchain@stable`. No `rust-toolchain.toml` file — the project doesn't use nightly features and should build on any recent stable.

### C compiler

`rusqlite` with `bundled` feature compiles SQLite from C source. Both `ubuntu-latest` and `macos-latest` have `cc` available by default. No extra setup needed.

### Action pinning policy

All third-party Actions are pinned to full commit SHAs with a version comment. `dtolnay/rust-toolchain` uses `@stable` as its intended interface (branch name, not tag). SHA pinning prevents supply-chain attacks via mutable tags.

---

## Release Workflow (`release.yml`)

### Triggers

```yaml
on:
  push:
    tags: ["v[0-9]+.[0-9]+.[0-9]+"]
```

Semver-only pattern prevents accidental releases from malformed tags.

### Permissions

```yaml
permissions:
  contents: write
```

Required for creating GitHub Releases.

### Matrix

| Target | Runner | Cross? | Notes |
|--------|--------|--------|-------|
| `x86_64-unknown-linux-gnu` | `ubuntu-latest` | No | Native compile |
| `aarch64-unknown-linux-gnu` | `ubuntu-latest` | Yes | Cross-compile via `cross` |
| `aarch64-apple-darwin` | `macos-latest` | No | Native on Apple Silicon runners |

Three targets. macOS x86_64 is dropped — `macos-latest` is Apple Silicon, and cross-compiling C code (rusqlite bundled SQLite) to x86_64 from ARM is fragile. Intel Mac users can build from source or run via Rosetta.

### Steps per target

1. **Checkout** — `actions/checkout` (SHA-pinned)
2. **Install Rust toolchain** — `dtolnay/rust-toolchain@stable` with target added
3. **Install cross** (Linux aarch64 only) — `cargo install cross --locked --version 0.2.5`
4. **Build** — `cargo build --release --locked --target $TARGET` (or `cross build` for aarch64-linux). `--locked` ensures reproducible builds from `Cargo.lock`.
5. **Package** — `chmod +x` then tar.gz the binary: `local-memory-mcp-{target}.tar.gz`
6. **Upload artifact** — `actions/upload-artifact` (SHA-pinned)

### Timeout

`timeout-minutes: 30` on build jobs. Cross-compilation with Docker pull can be slow.

### Create Release job

Runs after all build jobs complete:

1. **Download all artifacts** — `actions/download-artifact` (SHA-pinned)
2. **Generate checksums** — `sha256sum *.tar.gz > SHA256SUMS.txt`
3. **Create GitHub Release** — uses `gh release create` (GitHub CLI, no third-party Action) with all `.tar.gz` files and checksums. Uses `--draft` first, then publishes after verifying all assets are present.

### Why `gh` CLI instead of `softprops/action-gh-release`?

The `gh` CLI is pre-installed on all GitHub runners and uses the built-in `GITHUB_TOKEN`. No third-party Action needed, eliminating a supply-chain dependency that had `contents: write` access.

### Why `cross` for Linux aarch64?

Cross-compiling `rusqlite` with `bundled` SQLite requires a C cross-compiler. `cross` provides a Docker-based toolchain that handles this transparently.

### Why not macOS x86_64?

`macos-latest` is Apple Silicon. Cross-compiling C code (rusqlite bundled SQLite) to x86_64 from ARM requires explicit toolchain configuration and is fragile. Intel Mac market share is declining. Users can build from source or use Rosetta.

### fail-fast

`fail-fast: false` on the release matrix so all targets build independently. If one fails, you still get artifacts from the others and can see exactly which target broke.

---

## What We Do NOT Include

- **Windows builds** — no Windows-specific code, no install script, no user demand
- **macOS x86_64 builds** — fragile cross-compile from ARM runners, declining user base
- **Nightly toolchain** — no nightly features used
- **Code coverage** — adds complexity, low value for a personal project at this stage
- **Dependency auditing** — `cargo audit` is useful but can be added later as a backlog item
- **Docker image** — the project is a single binary, not a containerized service
- **CD/auto-deploy** — it's a local tool, not a deployed service
- **SLSA provenance attestation** — overkill for a personal project; add when the project gains users

---

## File Structure

```
.github/
└── workflows/
    ├── ci.yml        # Push/PR checks: fmt, clippy, test
    └── release.yml   # Tag-triggered release builds
```

---

## Implementation Plan

### Task 1: Create `ci.yml`
- Create `.github/workflows/ci.yml`
- Matrix: ubuntu-latest, macos-latest
- Steps: checkout, toolchain, cache, fmt, clippy, test
- All Actions SHA-pinned

### Task 2: Create `release.yml`
- Create `.github/workflows/release.yml`
- Matrix: 3 targets (linux x86_64, linux aarch64, macos aarch64)
- Build steps with cross for linux aarch64, `--locked` on all builds
- Create Release job using `gh` CLI with draft-then-publish pattern

---

## DAG

```
Task 1 (ci.yml)
Task 2 (release.yml)
```

Tasks 1 and 2 are independent — no dependencies between them.

---

## Sub-Agent Instructions

### Task 1: Create `ci.yml`

Create `.github/workflows/ci.yml`:

```yaml
name: CI

on:
  push:
    branches: [main]
  pull_request:
    branches: [main]

permissions:
  contents: read

concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true

jobs:
  check:
    name: Check (${{ matrix.os }})
    runs-on: ${{ matrix.os }}
    timeout-minutes: 15
    strategy:
      matrix:
        os: [ubuntu-latest, macos-latest]
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - uses: dtolnay/rust-toolchain@stable
        with:
          components: rustfmt, clippy
      - uses: Swatinem/rust-cache@9d47c6ad4b02e050fd481d890b2ea34778fd09d6 # v2.7.8
      - run: cargo fmt --check
      - run: cargo clippy --all-targets -- -D warnings
      - run: cargo test
```

### Task 2: Create `release.yml`

Create `.github/workflows/release.yml`:

```yaml
name: Release

on:
  push:
    tags: ["v[0-9]+.[0-9]+.[0-9]+"]

permissions:
  contents: write

jobs:
  build:
    name: Build (${{ matrix.target }})
    runs-on: ${{ matrix.runner }}
    timeout-minutes: 30
    strategy:
      fail-fast: false
      matrix:
        include:
          - target: x86_64-unknown-linux-gnu
            runner: ubuntu-latest
            cross: false
          - target: aarch64-unknown-linux-gnu
            runner: ubuntu-latest
            cross: true
          - target: aarch64-apple-darwin
            runner: macos-latest
            cross: false
    steps:
      - uses: actions/checkout@11bd71901bbe5b1630ceea73d27597364c9af683 # v4.2.2
      - uses: dtolnay/rust-toolchain@stable
        with:
          targets: ${{ matrix.target }}
      - name: Install cross
        if: matrix.cross
        run: cargo install cross --locked --version 0.2.5
      - name: Build
        env:
          USE_CROSS: ${{ matrix.cross }}
          TARGET: ${{ matrix.target }}
        run: |
          if [ "$USE_CROSS" = "true" ]; then
            cross build --release --locked --target "$TARGET"
          else
            cargo build --release --locked --target "$TARGET"
          fi
      - name: Package
        env:
          TARGET: ${{ matrix.target }}
        run: |
          cd "target/$TARGET/release"
          chmod +x local-memory-mcp
          tar czf "../../../local-memory-mcp-$TARGET.tar.gz" local-memory-mcp
      - uses: actions/upload-artifact@ea165f8d65b6e75b540449e92b4886f43607fa02 # v4.6.2
        with:
          name: local-memory-mcp-${{ matrix.target }}
          path: local-memory-mcp-${{ matrix.target }}.tar.gz

  release:
    name: Create Release
    needs: build
    runs-on: ubuntu-latest
    timeout-minutes: 10
    steps:
      - uses: actions/download-artifact@d3f86a106a0bac45b974a628896c90dbdf5c8093 # v4.3.0
        with:
          merge-multiple: true
      - name: Checksums
        run: sha256sum *.tar.gz > SHA256SUMS.txt
      - name: Create release
        env:
          GH_TOKEN: ${{ github.token }}
          TAG: ${{ github.ref_name }}
        run: |
          gh release create "$TAG" --draft --generate-notes --repo "$GITHUB_REPOSITORY"
          gh release upload "$TAG" *.tar.gz SHA256SUMS.txt --repo "$GITHUB_REPOSITORY"
          gh release edit "$TAG" --draft=false --repo "$GITHUB_REPOSITORY"
```
