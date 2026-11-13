#!/usr/bin/env bash
# Cache only locked registry/Git dependency compilations. Workspace/path crates
# always reach the actual compiler: their generated API and local inputs must be
# rebuilt in this job's private output, irrespective of another branch's timestamps.
set -euo pipefail
compiler=${1:?Cargo must supply its compiler}; shift
direct() {
  if [ -n "${REMOTE_CACHE_TRACE:-}" ]; then
    printf 'direct %s\n' "${CARGO_MANIFEST_DIR:-probe}" >> "$REMOTE_CACHE_TRACE"
  fi
  exec "$compiler" "$@"
}
[ "${REMOTE_DEPENDENCY_CACHE:-0}" = 1 ] || direct "$@"
[ "$(basename "$compiler")" = rustc ] || direct "$@" # e.g. nested clippy-driver
[ "${CARGO_INCREMENTAL:-0}" != 1 ] && [ "${CARGO_BUILD_INCREMENTAL:-0}" != 1 ] || direct "$@"
manifest=$(realpath -e -- "${CARGO_MANIFEST_DIR:-/nonexistent}" 2>/dev/null) || direct "$@"
cargo_home=$(realpath -e -- "${CARGO_HOME:-$HOME/.cargo}" 2>/dev/null) || direct "$@"
eligible=false
for tree in registry/src git/checkouts; do
  root=$(realpath -e -- "$cargo_home/$tree" 2>/dev/null) || continue
  case "$manifest/" in "$root/"*) eligible=true ;; esac
done
[ "$eligible" = true ] || direct "$@"

# Preserve cwd/argv/environment. A source outside this public package is not an
# eligible dependency invocation, even if a caller supplies misleading metadata.
source=''
for argument in "$@"; do
  case "$argument" in
    -* ) ;;
    *.rs ) [ -z "$source" ] || direct "$@"; source=$argument ;;
  esac
done
[ -n "$source" ] || direct "$@"
source=$(realpath -e -- "$source" 2>/dev/null) || direct "$@"
case "$source" in "$manifest/"*) ;; *) direct "$@" ;; esac
if [ -n "${REMOTE_CACHE_TRACE:-}" ]; then
  printf 'dependency %s\n' "$manifest" >> "$REMOTE_CACHE_TRACE"
fi
exec /usr/local/bin/sccache "$compiler" "$@"
