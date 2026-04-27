#!/usr/bin/env bash
set -euo pipefail
# Requires bash — do not change shebang to /bin/sh

main() {
  local REPO="chriswessells/local-memory-mcp"
  local BINARY="local-memory-mcp"
  local INSTALL_DIR="${INSTALL_DIR:-$HOME/.local/bin}"
  local WORK_DIR=""

  cleanup() {
    [ -n "$WORK_DIR" ] && rm -rf "$WORK_DIR"
    rm -f "$INSTALL_DIR/.$BINARY.tmp"
  }
  trap cleanup EXIT INT TERM

  # Platform detection
  local OS ARCH TARGET
  OS=$(uname -s)
  ARCH=$(uname -m)
  case "${OS}-${ARCH}" in
    Linux-x86_64)   TARGET="x86_64-unknown-linux-gnu" ;;
    Linux-aarch64)   TARGET="aarch64-unknown-linux-gnu" ;;
    Darwin-arm64)    TARGET="aarch64-apple-darwin" ;;
    *)
      echo "error: unsupported platform: ${OS}-${ARCH}"
      echo "Supported: Linux x86_64, Linux aarch64, macOS arm64"
      echo "Build from source: cargo install --path ."
      exit 1
      ;;
  esac

  # Find download tool
  local CURL="" WGET=""
  if command -v curl >/dev/null 2>&1; then
    CURL=1
  elif command -v wget >/dev/null 2>&1; then
    WGET=1
  else
    echo "error: curl or wget is required"
    exit 1
  fi

  download() {
    if [ -n "$CURL" ]; then
      curl --proto '=https' --tlsv1.2 -fsSL "$1" -o "$2"
    else
      wget --secure-protocol=TLSv1_2 -O "$2" "$1"
    fi
  }

  WORK_DIR=$(mktemp -d)
  local TARBALL="local-memory-mcp-${TARGET}.tar.gz"
  local BASE_URL="https://github.com/${REPO}/releases/latest/download"

  echo "Downloading ${BINARY} for ${TARGET}..."
  download "${BASE_URL}/${TARBALL}" "${WORK_DIR}/${TARBALL}" || {
    echo "error: download failed"
    echo "Manual download: ${BASE_URL}/${TARBALL}"
    exit 1
  }

  download "${BASE_URL}/SHA256SUMS.txt" "${WORK_DIR}/SHA256SUMS.txt" || {
    echo "error: failed to download checksums"
    exit 1
  }

  # Verify checksum
  local EXPECTED ACTUAL
  EXPECTED=$(grep -F "$TARBALL" "$WORK_DIR/SHA256SUMS.txt" | awk '{print $1}')
  if [ -z "$EXPECTED" ]; then
    echo "error: checksum not found for ${TARBALL}"
    exit 1
  fi

  if command -v sha256sum >/dev/null 2>&1; then
    ACTUAL=$(sha256sum "$WORK_DIR/$TARBALL" | awk '{print $1}')
  else
    ACTUAL=$(shasum -a 256 "$WORK_DIR/$TARBALL" | awk '{print $1}')
  fi

  if [ "$EXPECTED" != "$ACTUAL" ]; then
    echo "error: checksum mismatch"
    echo "  expected: ${EXPECTED}"
    echo "  actual:   ${ACTUAL}"
    exit 1
  fi

  # Extract and verify
  # Extract only the expected binary
  tar xzf "$WORK_DIR/$TARBALL" -C "$WORK_DIR" "$BINARY"
  if [ ! -f "$WORK_DIR/$BINARY" ]; then
    echo "error: binary not found in archive"
    exit 1
  fi

  # Atomic install
  mkdir -p "$INSTALL_DIR"
  cp "$WORK_DIR/$BINARY" "$INSTALL_DIR/.$BINARY.tmp"
  chmod +x "$INSTALL_DIR/.$BINARY.tmp"
  mv "$INSTALL_DIR/.$BINARY.tmp" "$INSTALL_DIR/$BINARY"

  echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"

  # PATH check
  case ":$PATH:" in
    *":$INSTALL_DIR:"*) ;;
    *) echo "Add to PATH: export PATH=\"${INSTALL_DIR}:\$PATH\"" ;;
  esac

  # MCP config
  cat <<EOF

Add to your MCP config:

{
  "mcpServers": {
    "local-memory": {
      "command": "${INSTALL_DIR}/${BINARY}",
      "args": []
    }
  }
}
EOF
}

main "$@"
