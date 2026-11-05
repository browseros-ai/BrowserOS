#!/usr/bin/env bash
# claw-sdk's gate. Copy to apps/claw-sdk/check.sh; it is also the package's `lint:rust`.
#
# It checks what the workspace-wide CI commands do not: Google's rustfmt options (stable rustfmt
# applies them only through --config), the dependency boundary, and the exact CI toolchain. The
# [lints] table and clippy.toml need nothing here, because Cargo and clippy read them on every
# build, including CI's `cargo clippy --workspace`.
#
# It covers every crate on the claw-sdk lint set: apps/claw-sdk, and crates/browseros-playwright
# once that crate exists.
#
# In a Grove worktree, run it with a private build dir, or it waits behind other workers:
#   CARGO_BUILD_BUILD_DIR="$(git rev-parse --show-toplevel)/packages/browseros-agent/target/build-private" \
#     apps/claw-sdk/check.sh
set -euo pipefail

AGENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)" # packages/browseros-agent
REPO_DIR="$(cd "$AGENT_DIR/../.." && pwd)"
cd "$AGENT_DIR"

# The toolchain CI pins in test.yml. Clippy's lints change between releases, so the gate runs CI's
# exact one, whatever `stable` is locally. The grep fails as soon as test.yml moves to another pin.
TOOLCHAIN="1.98.0"
if ! grep -q "dtolnay/rust-toolchain@${TOOLCHAIN}" "$REPO_DIR/.github/workflows/test.yml"; then
  echo "check.sh: .github/workflows/test.yml no longer pins ${TOOLCHAIN}; update TOOLCHAIN here." >&2
  exit 1
fi
if ! cargo "+${TOOLCHAIN}" clippy --version >/dev/null 2>&1; then
  echo "check.sh: toolchain ${TOOLCHAIN} is missing. Install it with:" >&2
  echo "  rustup toolchain install ${TOOLCHAIN} --profile minimal --component clippy,rustfmt" >&2
  exit 1
fi
cargo_pinned() { cargo "+${TOOLCHAIN}" "$@"; }

PACKAGES=(claw-sdk)
if [[ -f crates/browseros-playwright/Cargo.toml ]]; then
  PACKAGES+=(browseros-playwright)
fi
PACKAGE_ARGS=()
for package in "${PACKAGES[@]}"; do
  PACKAGE_ARGS+=(-p "$package")
done

step() { printf '\n== %s\n' "$1"; }

# Google's three options beyond the defaults. rustfmt.toml ignores them on stable; --config applies
# them (verified on rustfmt 1.9.0, toolchains 1.96.1 and 1.98.0). Plain `cargo fmt` keeps their
# output unchanged, so CI's `cargo fmt --all --check` agrees with this one.
FMT_CONFIG="imports_granularity=Module,hex_literal_case=Lower,format_macro_matchers=true"
step "rustfmt (${FMT_CONFIG})"
cargo_pinned fmt "${PACKAGE_ARGS[@]}" -- --check --config "$FMT_CONFIG"

# Crates that must never enter the graph (normal and build edges, transitively). claw-sdk builds on
# browseros-cdp/-core and new code (CONTEXT.md); a dev-dependency of a dependency never counts.
FORBIDDEN=(
  "claw-server:the desktop app; claw-sdk is a separate product"
  "browseros-mcp:today's run tool and InnerCallHook; prior art only, the dispatcher replaces them"
  "harness-integrations:desktop harness code"
  "claw-api:the desktop cockpit contract"
  "hosted-api:the old hosted /v1 contract; claw-sdk serves claw-sdk-api"
  "openssl-sys:the workspace is rustls-only, and the glibc 2.17 zigbuild target has no OpenSSL"
  "native-tls:same as openssl-sys"
  "boa_engine:one JS engine (rquickjs), owned by runtime/"
  "v8:one JS engine (rquickjs), owned by runtime/"
  "deno_core:one JS engine (rquickjs), owned by runtime/"
  "mozjs:one JS engine (rquickjs), owned by runtime/"
)
step "dependencies"
failed=0
for package in "${PACKAGES[@]}"; do
  graph="$(cargo_pinned tree -p "$package" --locked -e normal,build --prefix none --format '{p}' | awk '{print $1}' | sort -u)"
  for entry in "${FORBIDDEN[@]}"; do
    crate="${entry%%:*}"
    if grep -qx "$crate" <<<"$graph"; then
      echo "  $package depends on $crate (${entry#*:}). See: cargo tree -p $package -i $crate" >&2
      failed=1
    fi
  done
done
if [[ "$failed" -ne 0 ]]; then
  exit 1
fi
echo "  ok"

step "clippy -D warnings"
cargo_pinned clippy "${PACKAGE_ARGS[@]}" --all-targets --locked -- -D warnings

step "tests"
cargo_pinned test "${PACKAGE_ARGS[@]}" --locked

printf '\ncheck.sh: all checks passed\n'
