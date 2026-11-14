#!/usr/bin/env bash
# The Neo SDK gate, also called by the server package's `lint:rust`.
#
# It covers every Rust crate under crates/neo-sdk, read from the folder, so a new crate is in the
# gate the day it lands. Two kinds, by the crate's own Cargo.toml:
#   - claw-sdk's lint set: a restated [lints] table (neo-server, neo-playwright). On top of the
#     rest, Google's rustfmt options (stable rustfmt applies them only through --config) and the
#     dependency boundary.
#   - the workspace's lints (`[lints] workspace = true`: neo-takeover, neo-trace, neo-http,
#     ...): plain rustfmt, as CI's `cargo fmt --all --check` and their own `lint:rust` run it.
# Every crate gets clippy -D warnings on all targets and its tests (except TESTS_NOT_RUN below),
# on CI's exact toolchain. The SDK image's Python scripts (neo-server/docker) get their tests and
# an import each, and the pinned ruff where uv is installed, as CI's image scripts job does. None of these tests needs a display or a browser: the live suites skip
# themselves (`#[ignore]`, or no `CLAW_SDK_E2E_BROWSER`), as CI's `cargo test --workspace` runs
# them. The [lints] tables and clippy.toml need nothing here, because Cargo and clippy read them
# on every build.
#
# In a Grove worktree, run it with a private build dir, or it waits behind other workers:
#   CARGO_BUILD_BUILD_DIR="$(git rev-parse --show-toplevel)/packages/browseros-agent/target/build-private" \
#     crates/neo-sdk/check.sh
set -euo pipefail

AGENT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)" # packages/browseros-agent
REPO_DIR="$(cd "$AGENT_DIR/../.." && pwd)"
cd "$AGENT_DIR"

python3 "$AGENT_DIR/crates/neo-sdk/check-public-names.py" "$REPO_DIR"

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

# PACKAGES: claw-sdk's lint set. WORKSPACE_LINTED: every other SDK crate. ALL_ARGS: both.
PACKAGES=()
WORKSPACE_LINTED=()
for manifest in crates/neo-sdk/*/Cargo.toml; do
  package="$(sed -n 's/^name = "\(.*\)"$/\1/p' "$manifest" | head -n 1)"
  if [[ -z "$package" ]]; then
    echo "check.sh: no package name in $manifest" >&2
    exit 1
  fi
  # claw-sdk's lint set restates its tables; the workspace's is one `workspace = true`. A crate
  # with neither has no lints at all, which no SDK crate may.
  if grep -qx '\[lints\.rust\]' "$manifest" && grep -qx '\[lints\.clippy\]' "$manifest"; then
    PACKAGES+=("$package")
  elif grep -A1 -x '\[lints\]' "$manifest" | grep -qx 'workspace = true'; then
    WORKSPACE_LINTED+=("$package")
  else
    echo "check.sh: $manifest has no lint table: restate claw-sdk's [lints.rust] and [lints.clippy], or add [lints] workspace = true." >&2
    exit 1
  fi
done
for required in neo-server neo-playwright; do
  if [[ " ${PACKAGES[*]} " != *" $required "* ]]; then
    echo "check.sh: $required is not on claw-sdk's lint set (its own [lints] table); fix the table or this script." >&2
    exit 1
  fi
done
PACKAGE_ARGS=()
for package in "${PACKAGES[@]}"; do
  PACKAGE_ARGS+=(-p "$package")
done
WORKSPACE_ARGS=()
for package in "${WORKSPACE_LINTED[@]}"; do
  WORKSPACE_ARGS+=(-p "$package")
done
ALL_ARGS=("${PACKAGE_ARGS[@]}" "${WORKSPACE_ARGS[@]}")

# Crates whose tests the gate does not run yet; rustfmt and clippy still cover them. Each entry
# names why and what brings it back. CI's `cargo test --workspace` still runs them (one retry).
# - neo-captcha: two tests are flaky outside this gate's control, measured 2026-11-12 on its own
#   `cargo test -p neo-captcha`: events::tests_that_tracing_records_contract_fields_without_sensitive_data
#   fails 2 runs in 5 in parallel (a thread-local subscriber against tracing's shared callsite
#   interest), and twocaptcha::tests_that_cancel_mid_poll_and_deadlines_honor_paid_submission
#   1 in 6 single-threaded (a deadline). Drop the entry once both are fixed.
TESTS_NOT_RUN=(neo-captcha)
TEST_ARGS=()
for package in "${PACKAGES[@]}" "${WORKSPACE_LINTED[@]}"; do
  if [[ " ${TESTS_NOT_RUN[*]} " != *" $package "* ]]; then
    TEST_ARGS+=(-p "$package")
  fi
done
printf 'claw-sdk lint set: %s\nworkspace lints:   %s\n' "${PACKAGES[*]}" "${WORKSPACE_LINTED[*]:-none}"

step() { printf '\n== %s\n' "$1"; }

# Google's three options beyond the defaults. rustfmt.toml ignores them on stable; --config applies
# them (verified on rustfmt 1.9.0, toolchains 1.96.1 and 1.98.0). Plain `cargo fmt` keeps their
# output unchanged, so CI's `cargo fmt --all --check` agrees with this one.
FMT_CONFIG="imports_granularity=Module,hex_literal_case=Lower,format_macro_matchers=true"
step "rustfmt (${FMT_CONFIG})"
cargo_pinned fmt "${PACKAGE_ARGS[@]}" -- --check --config "$FMT_CONFIG"

# The workspace-linted crates keep plain rustfmt: their own `lint:rust` and CI check exactly this.
if [[ ${#WORKSPACE_ARGS[@]} -gt 0 ]]; then
  step "rustfmt (workspace defaults)"
  cargo_pinned fmt "${WORKSPACE_ARGS[@]}" -- --check
fi

# Crates that must never enter the graph (normal and build edges, transitively). claw-sdk builds on
# browseros-cdp/-core and new code (CONTEXT.md); a dev-dependency of a dependency never counts.
FORBIDDEN=(
  "claw-server:the desktop app; claw-sdk is a separate product"
  "browseros-mcp:today's run tool and InnerCallHook; prior art only, the dispatcher replaces them"
  "harness-integrations:desktop harness code"
  "claw-api:the desktop cockpit contract"
  "hosted-api:the old hosted /v1 contract; claw-sdk serves neo-sdk-api"
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

# The SDK image's Python scripts (neo-server/docker): nothing above builds or imports them, and
# CI's "Neo SDK image / scripts" job checks them, so a broken one used to pass this gate and fail
# only on GitHub. Their tests as that job runs them, then a load of every script, subfolders
# included, which fails as the image would on a name a script uses before it defines it. That job's ruff is
# packages/browseros's pinned one, through uv: it runs here when uv is installed; the remote pool
# has none, so there the gate names the command to run by hand.
DOCKER_DIR="crates/neo-sdk/neo-server/docker"
step "image scripts (${DOCKER_DIR})"
if ! python3 -c 'import sys; sys.exit(sys.version_info < (3, 12))'; then
  echo "check.sh: the image scripts need Python 3.12 or later, as the image runs them; python3 here is $(python3 --version 2>&1)." >&2
  exit 1
fi
(
  cd "$DOCKER_DIR"
  python3 -m unittest discover -s supervisor -p "*_test.py" -q
  python3 -m unittest discover -s . -p "*_test.py" -q
  # Every script, in subfolders too, by its path: a name like neo-quit.py is no module name.
  # Each loads as a module, so its `if __name__ == "__main__"` part does not run.
  python3 -I - <<'PY'
import importlib.util
import pathlib
import sys

scripts = sorted(p for p in pathlib.Path(".").rglob("*.py") if not p.name.endswith("_test.py"))
for path in scripts:
    sys.path.insert(0, str(path.parent))  # a script imports the scripts beside it
    name = "gate_" + "_".join(path.with_suffix("").parts).replace("-", "_")
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
print(f"  {len(scripts)} scripts import")
PY
  if command -v uv >/dev/null 2>&1; then
    uv run --quiet --directory "$REPO_DIR/packages/browseros" ruff check "$PWD"
  else
    echo "  ruff not run: no uv here. When a ${DOCKER_DIR}/*.py file changed, run it as CI does:"
    echo "    cd packages/browseros-agent/${DOCKER_DIR} && uv run --directory ../../../../../browseros ruff check \"\$PWD\""
  fi
)

step "clippy -D warnings"
cargo_pinned clippy "${ALL_ARGS[@]}" --all-targets --locked -- -D warnings

step "tests (not run: ${TESTS_NOT_RUN[*]})"
cargo_pinned test "${TEST_ARGS[@]}" --locked

# The embedded host is off in the default graph. Shared startup or constructor changes must
# still compile and pass the same lints and tests through that composition root.
step "clippy hosted-run -D warnings"
cargo_pinned clippy -p neo-server --all-targets --locked --features hosted-run -- -D warnings

step "tests hosted-run"
cargo_pinned test -p neo-server --locked --features hosted-run

printf '\ncheck.sh: all checks passed\n'
