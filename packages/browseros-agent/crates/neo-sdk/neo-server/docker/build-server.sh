#!/usr/bin/env bash
# The Dockerfile's server step: compiles browseros-claw-sdk from the sources copied into /src and
# proves the binary was compiled from exactly those sources.
#
#   build-server.sh <rust target triple>      writes /out/browseros-claw-sdk and /out/source-digest
#
# Why: /src/target is a BuildKit cache mount shared by every checkout built on the Docker daemon,
# so the third-party crates (about 365) compile once. Cargo decides whether a crate is fresh by
# comparing file mtimes, so a checkout whose sources are older than the cache's last build looked
# fresh and shipped that build's server (same sha256) under this checkout's revision. So:
#
#   1. the workspace's own packages are cleaned from the cache before every build (third-party
#      output and the registry stay shared): cargo must compile them from these sources, whatever
#      the clocks say. Touching the sources instead would still trust mtimes (a VM clock can step
#      back), and cargo's checksum freshness is nightly-only.
#   2. the build is checked, not trusted: every workspace crate must be reported compiled in this
#      step (not "fresh"), and the binary must carry this build's source digest, compiled in from
#      NEO_SDK_SOURCE_MARKER (cargo rebuilds a crate when an env var it reads with option_env!
#      changes). provenance.json records the digest and build.sh checks the image's binary again.
set -euo pipefail

target=${1:?build-server.sh needs a target triple}
cd /src

# The digest of every file this build compiles from (the context build.sh sent, as copied), by
# path and content. Cargo's output directory is not a source.
digest=$(find . -path ./target -prune -o -type f -print0 | LC_ALL=C sort -z | xargs -0 sha256sum | sha256sum | cut -c1-64)
marker="browseros-claw-sdk-source=$digest;"
echo "build-server.sh: sources $digest" >&2

# `cargo clean -p` resolves the dependency graph, so the registry must hold it first (a no-op
# once the shared registry cache is warm). Host and target are cleaned apart: build scripts are
# compiled for the host, under target/release.
cargo fetch --locked
members=$(cargo metadata --no-deps --offline --format-version 1 \
  | python3 -c 'import json, sys; print(" ".join("-p " + p["name"] for p in json.load(sys.stdin)["packages"]))')
# shellcheck disable=SC2086 # one -p NAME pair per member
cargo clean --release --offline $members
# shellcheck disable=SC2086
cargo clean --release --offline --target "$target" $members

NEO_SDK_SOURCE_MARKER="$marker" cargo zigbuild --release --locked --target "$target.2.17" \
  -p neo-server --bin browseros-claw-sdk --message-format=json-render-diagnostics >/tmp/cargo-messages.jsonl
binary="target/$target/release/browseros-claw-sdk"

# Every workspace crate in the server's graph was compiled now, none reused from the cache.
python3 - /tmp/cargo-messages.jsonl <<'PY'
import json, sys
compiled, reused = set(), set()
for line in open(sys.argv[1]):
    if not line.startswith("{"):
        continue  # cargo-zigbuild's own notes, if any
    message = json.loads(line)
    if message.get("reason") != "compiler-artifact" or "path+file://" not in message["package_id"]:
        continue
    (reused if message["fresh"] else compiled).add(message["target"]["name"])
if reused or not compiled:
    sys.exit(f"build-server.sh: workspace crates reused from the shared cache, not compiled from these sources: {sorted(reused) or 'none compiled'}")
print(f"build-server.sh: compiled {len(compiled)} workspace crates from these sources", file=sys.stderr)
PY

built=$(LC_ALL=C grep -aoE 'browseros-claw-sdk-source=[0-9a-f]{64};' "$binary" | head -1 || true)
if [ "$built" != "$marker" ]; then
  echo "build-server.sh: the server binary carries '${built:-no source marker}', not this build's '$marker'" >&2
  exit 1
fi

install -D -m 0755 "$binary" /out/browseros-claw-sdk
printf '%s\n' "$digest" >/out/source-digest
