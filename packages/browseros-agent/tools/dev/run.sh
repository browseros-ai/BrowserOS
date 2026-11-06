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

make -sC "$DIR"
exec "$DIR/browseros-dev" "$@"
