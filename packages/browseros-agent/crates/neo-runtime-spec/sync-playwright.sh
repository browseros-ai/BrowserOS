#!/bin/sh
# Re-syncs spec/playwright/ from a Playwright checkout, byte for byte, then re-pins and
# regenerates. Usage: ./sync-playwright.sh <playwright-checkout>
#
# Upgrading Playwright is: run this, read the diff of spec/playwright/ and of the generated
# src/ and ts/ files, then fix whatever stops compiling in neo-runtime and neo-runtime-injected.
set -eu

checkout=${1:?usage: sync-playwright.sh <playwright-checkout>}
crate=$(cd "$(dirname "$0")" && pwd)
target=$crate/spec/playwright

# The recorded commit must describe the copied bytes, so refuse local edits in the checkout.
if [ -n "$(git -C "$checkout" status --porcelain -- packages/protocol/spec LICENSE NOTICE)" ]; then
  echo "sync-playwright.sh: $checkout has uncommitted changes in its spec; commit or stash them" >&2
  exit 1
fi
commit=$(git -C "$checkout" rev-parse HEAD)

rm -f "$target"/*.yml
cp "$checkout"/packages/protocol/spec/*.yml "$checkout"/LICENSE "$checkout"/NOTICE "$target"/

# The translator owns the hash framing (codegen/spec_files.rs), so it writes UPSTREAM. The new
# spec no longer matches the committed generated code, which fails the library's freshness
# check; NEO_RUNTIME_SPEC_CODEGEN=1 builds the translator without that library code.
cd "$crate"
NEO_RUNTIME_SPEC_CODEGEN=1 cargo run --quiet -p neo-runtime-spec --example codegen -- --pin-playwright "$commit"
NEO_RUNTIME_SPEC_CODEGEN=1 cargo run --quiet -p neo-runtime-spec --example codegen
