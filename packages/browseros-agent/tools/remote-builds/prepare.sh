#!/usr/bin/env bash
# Prepare a task-owned Ubuntu worker after cloud-init installs Rust 1.98.0 and Crabbox 0.83.0.
# Run only on the dedicated pilot VMs. This grants trusted developers Docker/root-equivalent
# access; it is a shared build fleet, not a sandbox for untrusted contributions.
set -euo pipefail
here=$(cd -- "$(dirname -- "$0")" && pwd)
agent=$(cd -- "$here/../.." && pwd)
. /etc/os-release
[ "$ID:$VERSION_ID:$(uname -m):$(id -un)" = ubuntu:24.04:x86_64:buildrunner ] \
  || { echo 'prepare.sh requires the dedicated Ubuntu 24.04 amd64 buildrunner VM' >&2; exit 2; }
. "$HOME/.cargo/env"
rustc +1.98.0 --version
[ "$(/usr/local/bin/crabbox version)" = 0.83.0 ]
temporary=$(mktemp -d)
trap 'rm -rf "$temporary"' EXIT

# Apt verifies packages against Docker's repository signing key. Pin all four versions
# so every worker gets the same engine/build frontend rather than whatever is latest.
sudo install -d -m 0755 /etc/apt/keyrings
curl -fsSL https://download.docker.com/linux/ubuntu/gpg -o "$temporary/docker.asc"
sudo install -m 0644 "$temporary/docker.asc" /etc/apt/keyrings/docker.asc
printf '%s\n' 'deb [arch=amd64 signed-by=/etc/apt/keyrings/docker.asc] https://download.docker.com/linux/ubuntu noble stable' \
  | sudo tee /etc/apt/sources.list.d/docker.list >/dev/null
sudo apt-get update -q
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  docker-ce=5:29.9.3-1~ubuntu.24.04~noble \
  docker-ce-cli=5:29.9.3-1~ubuntu.24.04~noble \
  containerd.io=2.4.2-4~ubuntu.24.04~noble \
  docker-buildx-plugin=0.39.0-1~ubuntu.24.04~noble
sudo usermod -aG docker buildrunner
sudo systemctl enable --now docker.service
# The viewer's image must match the Mac's native architecture. Rust cross-compiles
# with Zig; the arm64 image's setup commands use this registered interpreter.
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends \
  qemu-user-static=1:8.2.2+ds-0ubuntu1.18 binfmt-support=2.2.2-7
sudo update-binfmts --enable qemu-aarch64
grep -q '^flags:.*F' /proc/sys/fs/binfmt_misc/qemu-aarch64 \
  || { echo 'arm64 binfmt registration needs the fix_binary flag' >&2; exit 1; }

curl -fsSL --retry 3 https://nodejs.org/dist/v26.13.0/node-v26.13.0-linux-x64.tar.xz -o "$temporary/node.tar.xz"
echo "d28a96af6c38596f6d9d97c0ee531c13d238d757742387d1928de822d959e997  $temporary/node.tar.xz" | sha256sum -c -
sudo install -d /opt/remote-builds/node-26.13.0
sudo tar -xf "$temporary/node.tar.xz" --strip-components=1 -C /opt/remote-builds/node-26.13.0
for executable in node npm npx; do sudo ln -sfn "/opt/remote-builds/node-26.13.0/bin/$executable" "/usr/local/bin/$executable"; done
curl -fsSL --retry 3 https://github.com/oven-sh/bun/releases/download/bun-v1.4.2/bun-linux-x64.zip -o "$temporary/bun.zip"
echo "36368faef7527875d5ffa52e53cd48021741f2a83eb6208a8dd64068d422a913  $temporary/bun.zip" | sha256sum -c -
unzip -q "$temporary/bun.zip" -d "$temporary/bun"
sudo install -m 0755 "$temporary/bun/bun-linux-x64/bun" /usr/local/bin/bun
[ "$(bun --version)" = 1.4.2 ]
[ "$(node --version)" = v26.13.0 ]

# The same locked browser and fonts as neo-spec.yml, without a developer's profile.
. "$agent/crates/neo-sdk/neo-server/docker/browser.lock"
curl -fsSL --retry 3 "$NEO_BROWSER_URL_AMD64" -o "$temporary/neo.deb"
echo "$NEO_BROWSER_SHA256_AMD64  $temporary/neo.deb" | sha256sum -c -
chmod 0755 "$temporary"
sudo DEBIAN_FRONTEND=noninteractive apt-get install -y --no-install-recommends "$temporary/neo.deb" \
  libasound2t64 fonts-liberation fonts-noto-core fonts-noto-color-emoji fonts-wqy-zenhei fonts-ipafont-gothic

# Helpers are copied from this audited source transfer. Mutable slot/job records live
# separately; reinstalling code never resets a busy slot or a cancellation fence.
sudo install -d -m 0755 /usr/local/lib/browseros-remote-builds
for helper in pool guest reaper; do
  sudo install -m 0644 "$here/$helper.py" "/usr/local/lib/browseros-remote-builds/$helper.py.new"
  sudo mv "/usr/local/lib/browseros-remote-builds/$helper.py.new" "/usr/local/lib/browseros-remote-builds/$helper.py"
done
for helper in recipes.sh source-metadata-compat.cjs; do
  sudo install -m 0644 "$here/$helper" "/usr/local/lib/browseros-remote-builds/$helper.new"
  sudo mv "/usr/local/lib/browseros-remote-builds/$helper.new" "/usr/local/lib/browseros-remote-builds/$helper"
done
sudo install -m 0644 "$agent/crates/neo-sdk/neo-server/docker/build.sh" \
  /usr/local/lib/browseros-remote-builds/build-image.sh.new
sudo mv /usr/local/lib/browseros-remote-builds/build-image.sh.new \
  /usr/local/lib/browseros-remote-builds/build-image.sh
sudo install -d -m 0700 -o buildrunner -g buildrunner /var/lib/browseros-remote-builds \
  /var/lib/browseros-remote-builds/jobs
sudo install -d -m 0755 -o buildrunner -g buildrunner /work/remote-builds/cache
docker --version
docker buildx version
/usr/bin/browserclaw --version
echo 'remote-builds: worker prepared; reconnect SSH for Docker group membership'
