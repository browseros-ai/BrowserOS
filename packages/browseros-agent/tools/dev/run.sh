#!/bin/bash
set -e

DIR="$(cd "$(dirname "$0")" && pwd)"

if ! command -v go &>/dev/null; then
  echo ""
  echo "  Go is required to build browseros-dev but is not installed."
  if command -v brew &>/dev/null; then
    echo "  Install it with:  brew install go"
  else
    echo "  Install it with your package manager (e.g. apt install golang-go)"
  fi
  echo "  Or download from: https://go.dev/dl/"
  echo ""
  exit 1
fi

needs_cargo=false
has_claw=false
has_rust=false
if [ "${1:-}" = "watch" ]; then
  for arg in "$@"; do
    case "$arg" in
      --claw)
        has_claw=true
        ;;
      --rust)
        has_rust=true
        ;;
    esac
  done
  if [ "$has_claw" = true ] && [ "$has_rust" = true ]; then
    needs_cargo=true
  fi
fi

if [ "$needs_cargo" = true ] && ! command -v cargo &>/dev/null; then
  echo ""
  echo "  Cargo is required for dev:claw-rust:watch but is not installed."
  if command -v brew &>/dev/null; then
    echo "  Install Rust with:  brew install rustup && rustup-init"
  else
    echo "  Install Rust with:  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
  fi
  echo "  Or download from: https://rustup.rs/"
  echo ""
  exit 1
fi

make -sC "$DIR"
exec "$DIR/browseros-dev" "$@"
