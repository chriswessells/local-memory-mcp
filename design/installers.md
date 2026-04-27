# Component 11: Installers — Detailed Design

## Problem

Users must clone the repo and run `cargo build --release` to get the binary. This requires a Rust toolchain, a C compiler (for bundled SQLite), and familiarity with Cargo. The release workflow produces platform-specific tarballs on GitHub Releases, but there's no easy way to download and install them.

## Solution

A single `install.sh` bash script that:
1. Detects the platform (OS + architecture)
2. Downloads the correct tarball from the latest GitHub Release
3. Verifies the SHA256 checksum
4. Installs the binary atomically to `~/.local/bin` (or a user-specified directory)
5. Prints setup instructions with absolute paths for MCP server configuration

No `install.ps1` — Windows is not a supported platform.

---

## Install Script (`install.sh`)

### Usage

Download and inspect first (recommended):

```bash
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/chriswessells/local-memory-mcp/main/install.sh -o install.sh
less install.sh
bash install.sh
```

Or one-liner:

```bash
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/chriswessells/local-memory-mcp/main/install.sh | bash
```

With a custom install directory:

```bash
curl --proto '=https' --tlsv1.2 -fsSL https://raw.githubusercontent.com/chriswessells/local-memory-mcp/main/install.sh | INSTALL_DIR=/usr/local/bin bash
```

### Script Structure

All logic is wrapped in a `main()` function called at the end of the script. This prevents partial execution if the download is truncated — bash won't call `main` until the entire script is downloaded.

```bash
#!/usr/bin/env bash
set -euo pipefail
# Requires bash — do not change shebang to /bin/sh

main() {
  # ... all logic here ...
}

main "$@"
```

### Platform Detection

| `uname -s` | `uname -m` | Target |
|-------------|------------|--------|
| `Linux` | `x86_64` | `x86_64-unknown-linux-gnu` |
| `Linux` | `aarch64` | `aarch64-unknown-linux-gnu` |
| `Darwin` | `arm64` | `aarch64-apple-darwin` |

Any other combination exits with an error listing supported platforms. macOS on Intel is not supported — the error message suggests building from source with `cargo install --path .`.

### Download

Use `curl` (preferred) or `wget` as fallback. Exit with error if neither is available.

Download URL pattern:
```
https://github.com/chriswessells/local-memory-mcp/releases/latest/download/local-memory-mcp-{TARGET}.tar.gz
```

Also download `SHA256SUMS.txt` from the same release.

The `download` function:
- curl: `curl --proto '=https' --tlsv1.2 -fsSL "$url" -o "$output"`
- wget: `wget -O "$output" "$url"` (no `-q` — let errors show on stderr)

### Checksum Verification

1. Download `SHA256SUMS.txt`
2. Extract the expected checksum using `grep -F "$TARBALL"` (fixed-string match)
3. Compute actual checksum: `sha256sum` (Linux) or `shasum -a 256` (macOS)
4. Compare — abort if mismatch, print both expected and actual values

Note: SHA256SUMS.txt is not cryptographically signed. The checksum guards against download corruption and CDN issues, not against a compromised GitHub release. This is an accepted limitation for a personal project.

### Installation (Atomic)

1. Extract the tarball to `$WORK_DIR` (temp directory)
2. Verify the binary exists: `[ -f "$WORK_DIR/$BINARY" ]`
3. Create `$INSTALL_DIR` if it doesn't exist: `mkdir -p "$INSTALL_DIR"`
4. Copy to temp location on target filesystem: `cp "$WORK_DIR/$BINARY" "$INSTALL_DIR/.$BINARY.tmp"`
5. `chmod +x "$INSTALL_DIR/.$BINARY.tmp"`
6. Atomic rename: `mv "$INSTALL_DIR/.$BINARY.tmp" "$INSTALL_DIR/$BINARY"`

The temp-then-rename pattern ensures the binary is never left in a truncated state. If interrupted, only the `.tmp` file is left behind (cleaned up by the EXIT trap).

### Post-Install Output

Print:
1. Installed path (absolute)
2. Whether `$INSTALL_DIR` is in `$PATH` — if not, print the `export PATH` command
3. MCP server configuration with **absolute path** (no tilde):

```
Add to your MCP config:

{
  "mcpServers": {
    "local-memory": {
      "command": "/home/user/.local/bin/local-memory-mcp",
      "args": []
    }
  }
}
```

The path is printed using `$INSTALL_DIR/$BINARY` which resolves to an absolute path.

PATH detection uses the colon-wrapping trick:
```bash
case ":$PATH:" in
  *":$INSTALL_DIR:"*) ;;
  *) echo "Add to PATH: export PATH=\"$INSTALL_DIR:\$PATH\"" ;;
esac
```

### Error Handling

- No `curl` or `wget` → error with install instructions
- Unsupported platform → error listing supported platforms
- Download failure → error with URL for manual download
- Checksum mismatch → error showing expected vs actual, suggest re-downloading
- Binary not found in archive → error
- No write permission to install dir → error suggesting `INSTALL_DIR` or `sudo`

### Cleanup

- `WORK_DIR=$(mktemp -d)` for all temp files
- `trap cleanup EXIT INT TERM` — covers normal exit, errors, Ctrl+C, and kill
- Cleanup also removes `$INSTALL_DIR/.$BINARY.tmp` if it exists

---

## What We Do NOT Include

- **`install.ps1`** — no Windows release binaries
- **Auto-update mechanism** — run the installer again to update
- **Package manager integration** (Homebrew, apt) — future backlog item
- **Version pinning** — always installs latest
- **Building from source** — installer only downloads pre-built binaries
- **Cryptographic signing** — SHA256 checksums guard against corruption only; signing is a future consideration if the project gains users
- **Installer CI testing** — defer to manual testing; add automated testing when the project grows

---

## File Structure

```
install.sh    # Root of repo, executable
```

---

## Implementation Plan

### Task 1: Create `install.sh`

Single task — the script is self-contained.

---

## DAG

```
Task 1 (install.sh)
```

---

## Sub-Agent Instructions

### Task 1: Create `install.sh`

Create `install.sh` in the repo root with `chmod +x`. The script must:

1. Start with `#!/usr/bin/env bash`, `set -euo pipefail`, and a comment: `# Requires bash — do not change shebang to /bin/sh`
2. Wrap all logic in a `main()` function, called at the end: `main "$@"`. This prevents partial execution on truncated downloads.
3. Define constants inside `main()`:
   ```bash
   REPO="chriswessells/local-memory-mcp"
   BINARY="local-memory-mcp"
   INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
   ```
4. Define `cleanup()` and register: `trap cleanup EXIT INT TERM`. Cleanup removes `$WORK_DIR` and `$INSTALL_DIR/.$BINARY.tmp`.
5. Detect platform with `uname -s` and `uname -m`. Map to TARGET via case statement. Error for unsupported combos — suggest `cargo install --path .` for unsupported platforms.
6. Find download tool (curl or wget). Define `download()`:
   - curl: `curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"`
   - wget: `wget -O "$2" "$1"`
7. Create temp directory: `WORK_DIR=$(mktemp -d)`
8. Download tarball and SHA256SUMS.txt to `$WORK_DIR`. Print what's being downloaded.
9. Verify checksum with `grep -F` and `sha256sum` or `shasum -a 256`. On mismatch, print expected vs actual and exit 1.
10. Extract: `tar xzf "$WORK_DIR/$TARBALL" -C "$WORK_DIR"`. Verify binary exists.
11. Atomic install:
    ```bash
    mkdir -p "$INSTALL_DIR"
    cp "$WORK_DIR/$BINARY" "$INSTALL_DIR/.$BINARY.tmp"
    chmod +x "$INSTALL_DIR/.$BINARY.tmp"
    mv "$INSTALL_DIR/.$BINARY.tmp" "$INSTALL_DIR/$BINARY"
    ```
12. Print success with absolute path, PATH check (colon-wrapping case statement), and MCP config JSON using `$INSTALL_DIR/$BINARY`.
