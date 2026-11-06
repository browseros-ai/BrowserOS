#!/usr/bin/env bash
# The gate for the neo crates (crates/NEO_STYLE.md): every Rust neo crate carries the same
# [lints] tables and passes fmt, clippy and its tests; neo-sdk typechecks and its committed
# bundle matches a fresh build. Covers whichever neo crates exist on this branch.
set -euo pipefail
cd "$(dirname "$0")/.."

# A manifest's [lints.*] tables, without comments or blank lines.
lint_tables() {
  awk '
    /^\[/ { in_lints = ($0 ~ /^\[lints(\.|\])/) }
    in_lints && !/^[[:space:]]*(#|$)/ { print }
  ' "$1"
}

manifests=(crates/neo-*/Cargo.toml)
reference=${manifests[0]}
for manifest in "${manifests[@]}"; do
  if ! diff -u --label "$reference" --label "$manifest" \
    <(lint_tables "$reference") <(lint_tables "$manifest") >&2; then
    echo "neo-check: the [lints] tables of $manifest differ from $reference" >&2
    exit 1
  fi
done

for manifest in "${manifests[@]}"; do
  crate=$(basename "$(dirname "$manifest")")
  echo "neo-check: $crate"
  cargo fmt --package "$crate" -- --check
  cargo clippy --package "$crate" --all-targets --locked -- -D warnings
  cargo test --package "$crate" --locked
done

echo "neo-check: neo-sdk"
(cd crates/neo-sdk && bun run typecheck && bun run build)
bundle=crates/neo-sdk/dist/sdk.js
if ! git ls-files --error-unmatch "$bundle" >/dev/null 2>&1 \
  || ! git diff --quiet -- "$bundle"; then
  echo "neo-check: commit $bundle as rebuilt from crates/neo-sdk/src" >&2
  git diff --stat -- "$bundle" >&2
  exit 1
fi
echo "neo-check: ok"
