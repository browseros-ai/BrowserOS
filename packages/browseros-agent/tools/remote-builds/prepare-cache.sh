#!/usr/bin/env bash
# Install the cache tool on a task-owned Linux worker. The job's systemd cgroup
# owns its foreground server and compiler children; only disk entries survive jobs.
set -euo pipefail
[ "$(id -un):$(uname -m)" = buildrunner:x86_64 ] || exit 2
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT
asset=sccache-v0.19.0-x86_64-unknown-linux-musl
curl -fLsS --retry 3 "https://github.com/mozilla/sccache/releases/download/v0.19.0/$asset.tar.gz" -o "$temporary/cache.tar.gz"
echo "aa73c7270c997894038c0d8f0d297865f57569e1687883322bc431713d4bb9dd  $temporary/cache.tar.gz" | sha256sum -c -
tar -xzf "$temporary/cache.tar.gz" -C "$temporary"
sudo install -m 0755 "$temporary/$asset/sccache" /usr/local/bin/sccache
[ "$(/usr/local/bin/sccache --version)" = 'sccache 0.19.0' ]
sudo install -d -m 0755 /etc/browseros-remote-builds
sudo install -d -m 0700 -o buildrunner -g buildrunner /work/remote-builds/cache/sccache
cat <<'CONFIG' | sudo tee /etc/browseros-remote-builds/sccache.toml >/dev/null
[cache.disk]
dir = "/work/remote-builds/cache/sccache"
size = 21474836480
CONFIG
echo 'Pinned sccache installed; dependency caching remains opt-in until benchmarked.'
