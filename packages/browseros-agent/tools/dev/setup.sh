#!/bin/bash
set -euo pipefail

DIR="$(cd "$(dirname "$0")" && pwd)"
exec "$DIR/run.sh" setup "$@"
