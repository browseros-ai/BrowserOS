#!/usr/bin/env bash
# The gate for the neo crates (crates/NEO_STYLE.md): every Rust neo crate carries the same
# [lints] tables and passes fmt, clippy and its tests; every neo TypeScript package typechecks,
# passes its `test` script, and its committed dist/ matches a fresh build. Covers whichever neo
# crates and packages exist on this branch, found by their manifests.
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

has_script() {
  bun -e "process.exit(require('./$1').scripts?.['$2'] ? 0 : 1)"
}

# Browser tests (`test:browser`, `test:parity`) stay out: they need NEO_E2E_BROWSER. A package
# with a `build` script commits its dist/, which Rust embeds with include_str!, so dist/ must be
# tracked and equal to a fresh build.
for package in crates/neo-*/package.json; do
  dir=$(dirname "$package")
  echo "neo-check: $(basename "$dir") (TypeScript)"
  for script in typecheck test; do
    if has_script "$package" "$script"; then
      (cd "$dir" && bun run "$script")
    fi
  done
  if has_script "$package" build; then
    (cd "$dir" && bun run build)
    if ! git ls-files --error-unmatch "$dir/dist" >/dev/null 2>&1 \
      || ! git diff --quiet -- "$dir/dist" \
      || [ -n "$(git ls-files --others --exclude-standard -- "$dir/dist")" ]; then
      echo "neo-check: commit $dir/dist as rebuilt from $dir/src" >&2
      git status --short -- "$dir/dist" >&2
      exit 1
    fi
  fi
done
echo "neo-check: ok"
