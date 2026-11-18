#!/usr/bin/env bash
# Fails when crates/neo-sdk/neo-sdk-api/openapi.yaml would break a client of the spec published at
# the merge-base with <base-ref> (default origin/main). Clients generate typed code from the
# spec, so oasdiff judges compatibility on the spec alone; run from anywhere in the repo.
#
# --fail-on WARN also fails on changes oasdiff cannot prove safe, such as removing a query
# parameter. oasdiff-levels.txt raises removing an optional response property to an error:
# the spec promises fields are never removed within /v1, and oasdiff's default lets it pass.
set -euo pipefail

base_ref="${1:-origin/main}"
image="tufin/oasdiff:v1.34.1"
repo="$(git rev-parse --show-toplevel)"
spec="packages/browseros-agent/crates/neo-sdk/neo-sdk-api/openapi.yaml"
levels="packages/browseros-agent/crates/neo-sdk/neo-sdk-api/oasdiff-levels.txt"
base="$(git -C "$repo" merge-base "$base_ref" HEAD)"

# A merge-base can predate the source move. Resolve its path independently so relocating the
# candidate never turns an existing compatibility comparison into a successful skip.
base_spec="$spec"
if ! git -C "$repo" cat-file -e "$base:$base_spec" 2>/dev/null; then
  base_spec="packages/browseros-agent/contracts/claw-sdk-api/openapi.yaml"
  if ! git -C "$repo" cat-file -e "$base:$base_spec" 2>/dev/null; then
    echo "No claw-sdk API spec at $base; nothing to compare against."
    exit 0
  fi
fi

work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT
git -C "$repo" show "$base:$base_spec" >"$work/base.yaml"
cp "$repo/$spec" "$work/revision.yaml"
cp "$repo/$levels" "$work/levels.txt"

docker run --rm -v "$work:/specs:ro" "$image" breaking \
  /specs/base.yaml /specs/revision.yaml \
  --fail-on WARN --severity-levels /specs/levels.txt
