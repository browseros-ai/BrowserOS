#!/bin/bash
set -e

DIR="$(cd "$(dirname "$0")" && pwd)"

if ! command -v go &>/dev/null; then
  echo ""
  echo "  Go is required to build browseros-dev but is not installed."
  echo "  Install it with:  brew install go"
  echo "  Or download from: https://go.dev/dl/"
  echo ""
  exit 1
fi

# watch runs a Rust server (BrowserOS or neo) unless --ts selects the TypeScript one.
needs_cargo=false
if [ "${1:-}" = "watch" ]; then
  needs_cargo=true
  for arg in "$@"; do
    if [ "$arg" = "--ts" ]; then
      needs_cargo=false
    fi
  done
fi

if [ "$needs_cargo" = true ] && ! command -v cargo &>/dev/null; then
  echo ""
  echo "  Cargo is required for watch (the Rust server) but is not installed."
  echo "  Pass --ts to run the TypeScript server instead."
  echo "  Install Rust with:  brew install rustup && rustup-init"
  echo "  Or download from: https://rustup.rs/"
  echo ""
  exit 1
fi

make -sC "$DIR"
exec "$DIR/browseros-dev" "$@"
