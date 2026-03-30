#!/usr/bin/env bash
#
# Install browseros-cli — downloads the latest release binary for your platform.
#
# Usage:
#   curl -fsSL https://cdn.browseros.com/cli/install.sh | bash
#
#   # Or with options:
#   curl -fsSL https://cdn.browseros.com/cli/install.sh | bash -s -- --version 0.1.0 --dir /usr/local/bin

set -euo pipefail

CDN_BASE="https://cdn.browseros.com/cli"
BINARY="browseros-cli"
INSTALL_DIR="${HOME}/.browseros/bin"

# ── Parse arguments ──────────────────────────────────────────────────────────

VERSION=""
CUSTOM_DIR=""

while [[ $# -gt 0 ]]; do
  case "$1" in
    --version)
      [[ $# -lt 2 ]] && { echo "Error: --version requires a value" >&2; exit 1; }
      VERSION="$2"; shift 2 ;;
    --dir)
      [[ $# -lt 2 ]] && { echo "Error: --dir requires a value" >&2; exit 1; }
      CUSTOM_DIR="$2"; shift 2 ;;
    --help)
      echo "Usage: install.sh [--version VERSION] [--dir INSTALL_DIR]"
      echo ""
      echo "  --version   Install a specific version (default: latest)"
      echo "  --dir       Install directory (default: ~/.browseros/bin)"
      exit 0
      ;;
    *) echo "Unknown option: $1" >&2; exit 1 ;;
  esac
done

[[ -n "$CUSTOM_DIR" ]] && INSTALL_DIR="$CUSTOM_DIR"

# ── Resolve latest version ───────────────────────────────────────────────────

if [[ -z "$VERSION" ]]; then
  VERSION=$(curl -fsSL "${CDN_BASE}/latest/version.txt" | tr -d '[:space:]')

  if [[ -z "$VERSION" ]]; then
    echo "Error: could not determine latest version." >&2
    echo "  Try: install.sh --version 0.1.0" >&2
    exit 1
  fi
fi

if [[ ! "$VERSION" =~ ^[0-9]+\.[0-9]+\.[0-9]+(-[a-zA-Z0-9.]+)?$ ]]; then
  echo "Error: unexpected version format: '$VERSION'" >&2
  exit 1
fi

echo "Installing browseros-cli v${VERSION}..."

# ── Detect platform ──────────────────────────────────────────────────────────

OS=$(uname -s | tr '[:upper:]' '[:lower:]')
ARCH=$(uname -m)

case "$OS" in
  darwin) OS="darwin" ;;
  linux)  OS="linux" ;;
  *)      echo "Error: unsupported OS: $OS" >&2; exit 1 ;;
esac

case "$ARCH" in
  x86_64|amd64) ARCH="amd64" ;;
  arm64|aarch64) ARCH="arm64" ;;
  *)             echo "Error: unsupported architecture: $ARCH" >&2; exit 1 ;;
esac

# ── Download and extract ─────────────────────────────────────────────────────

FILENAME="${BINARY}_${VERSION}_${OS}_${ARCH}.tar.gz"
URL="${CDN_BASE}/v${VERSION}/${FILENAME}"
CHECKSUM_URL="${CDN_BASE}/v${VERSION}/checksums.txt"

TMPDIR_DL=$(mktemp -d)
trap 'rm -rf "$TMPDIR_DL"' EXIT

echo "Downloading ${URL}..."
curl -fSL --progress-bar -o "${TMPDIR_DL}/${FILENAME}" "$URL"

# Verify checksum if sha256sum/shasum is available
if curl -fsSL -o "${TMPDIR_DL}/checksums.txt" "$CHECKSUM_URL" 2>/dev/null; then
  expected=$(awk -v filename="$FILENAME" '$2 == filename { print $1; exit }' "${TMPDIR_DL}/checksums.txt")
  if [[ -n "$expected" ]]; then
    if command -v sha256sum >/dev/null 2>&1; then
      actual=$(sha256sum "${TMPDIR_DL}/${FILENAME}" | awk '{print $1}')
    elif command -v shasum >/dev/null 2>&1; then
      actual=$(shasum -a 256 "${TMPDIR_DL}/${FILENAME}" | awk '{print $1}')
    else
      actual=""
      echo "Warning: no sha256sum/shasum found; skipping checksum verification." >&2
    fi
    if [[ -n "$actual" && "$actual" != "$expected" ]]; then
      echo "Error: checksum mismatch (expected ${expected}, got ${actual})" >&2
      exit 1
    fi
    [[ -n "$actual" ]] && echo "Checksum verified."
  else
    echo "Warning: checksum not found in checksums.txt; skipping verification." >&2
  fi
else
  echo "Warning: could not fetch checksums.txt; skipping checksum verification." >&2
fi

tar -xzf "${TMPDIR_DL}/${FILENAME}" -C "$TMPDIR_DL"

BINARY_PATH="${TMPDIR_DL}/${BINARY}"
if [[ ! -f "$BINARY_PATH" ]]; then
  BINARY_PATH=$(find "$TMPDIR_DL" -type f -name "$BINARY" -print -quit)
fi

if [[ -z "$BINARY_PATH" || ! -f "$BINARY_PATH" ]]; then
  echo "Error: binary not found in archive." >&2
  exit 1
fi

# ── Install ──────────────────────────────────────────────────────────────────

mkdir -p "$INSTALL_DIR"
mv "$BINARY_PATH" "${INSTALL_DIR}/${BINARY}"
chmod +x "${INSTALL_DIR}/${BINARY}"

echo "Installed ${BINARY} to ${INSTALL_DIR}/${BINARY}"

# ── PATH hint ────────────────────────────────────────────────────────────────

if ! echo "$PATH" | tr ':' '\n' | grep -qx "$INSTALL_DIR"; then
  echo ""
  echo "Add browseros-cli to your PATH:"
  echo ""

  SHELL_NAME=$(basename "${SHELL:-/bin/bash}")
  case "$SHELL_NAME" in
    zsh)  echo "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.zshrc && source ~/.zshrc" ;;
    fish) echo "  fish_add_path ${INSTALL_DIR}" ;;
    *)    echo "  echo 'export PATH=\"${INSTALL_DIR}:\$PATH\"' >> ~/.bashrc && source ~/.bashrc" ;;
  esac
fi

echo ""
echo "Run 'browseros-cli --help' to get started."
