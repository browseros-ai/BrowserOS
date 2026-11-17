#!/usr/bin/env bash
# Let a prepared pool worker host one-job GitHub Actions runners (README, "Run a job from
# GitHub"; COORDINATOR.md, "GitHub runners"). Run as buildrunner, after prepare.sh, from
# the same audited source copy, on a worker that holds no job. Running it again is safe.
#
# Usage: prepare-runner.sh DISPATCHER_PUBLIC_KEY_FILE COORDINATOR_PRIVATE_ADDRESS
set -euo pipefail
here=$(cd -- "$(dirname -- "$0")" && pwd)
. /etc/os-release
[ "$ID:$VERSION_ID:$(uname -m):$(id -un)" = ubuntu:24.04:x86_64:buildrunner ] \
  || { echo 'prepare-runner.sh requires a dedicated Ubuntu 24.04 amd64 buildrunner VM' >&2; exit 2; }
public_key=${1:?usage: prepare-runner.sh DISPATCHER_PUBLIC_KEY_FILE COORDINATOR_PRIVATE_ADDRESS}
coordinator=${2:?usage: prepare-runner.sh DISPATCHER_PUBLIC_KEY_FILE COORDINATOR_PRIVATE_ADDRESS}
[[ $coordinator =~ ^[0-9]{1,3}(\.[0-9]{1,3}){3}$ ]] || { echo 'coordinator must be its private IPv4 address' >&2; exit 2; }
read -r key_type key_body _ <"$public_key"
[ "$key_type" = ssh-ed25519 ] && [[ $key_body =~ ^[A-Za-z0-9+/]+=*$ ]] \
  || { echo 'expected the dispatcher public key (ssh-ed25519)' >&2; exit 2; }
version=2.338.2
sha256=baebf204307bac6193480852732f0aba2ae97b6c5597c244f4d5558390147c40
account=poolrunner
helpers=/usr/local/lib/browseros-remote-builds

# Member jobs run as this account: no password, no login shell, no sudo, and no group
# that grants Docker, LXD or log access. runner.sh gives each job its own home.
if ! id "$account" >/dev/null 2>&1; then
  sudo useradd --system --user-group --home-dir /nonexistent --no-create-home \
    --shell /usr/sbin/nologin "$account"
fi
[ "$(id -Gn "$account")" = "$account" ] || { echo "$account must belong to no other group" >&2; exit 1; }
# A cron job would outlive its runner job; `at` is not installed on the pool image.
sudo touch /etc/cron.deny
grep -qx "$account" /etc/cron.deny || echo "$account" | sudo tee -a /etc/cron.deny >/dev/null

dist=/opt/actions-runner/$version
if [ ! -x "$dist/run.sh" ]; then
  temporary=$(mktemp -d)
  trap 'rm -rf "$temporary"' EXIT
  curl -fsSL --retry 3 -o "$temporary/runner.tar.gz" \
    "https://github.com/actions/runner/releases/download/v$version/actions-runner-linux-x64-$version.tar.gz"
  echo "$sha256  $temporary/runner.tar.gz" | sha256sum -c -
  sudo rm -rf "$dist.new"
  sudo install -d -m 0755 /opt/actions-runner "$dist.new"
  sudo tar -xzf "$temporary/runner.tar.gz" -C "$dist.new" --no-same-owner
  sudo chmod -R go-w "$dist.new"
  sudo mv "$dist.new" "$dist"
fi
sudo ln -sfn "$dist" /opt/actions-runner/current

# guest.py carries the dispatcher's restricted entry; the two scripts run each job.
for helper in runner.sh runner-hook.sh guest.py; do
  sudo install -m 0755 "$here/$helper" "$helpers/$helper.new"
  sudo mv "$helpers/$helper.new" "$helpers/$helper"
done
sudo chmod 0644 "$helpers/guest.py"
# Per-job runner directories; the runner insists on reading every directory above its
# own. The pool's own job directories stay traversable but no longer listable, so
# retained developer sources are out of a runner job's reach.
sudo install -d -m 0755 -o root -g root /work/pool-runner
sudo chmod 0711 /work/remote-builds

# The dispatcher's key may only reach guest.py runner-rpc, and only from the coordinator.
line="from=\"$coordinator\",restrict,command=\"/usr/bin/python3 $helpers/guest.py runner-rpc\" $key_type $key_body pool-runner-dispatcher"
install -d -m 0700 ~/.ssh
touch ~/.ssh/authorized_keys
chmod 0600 ~/.ssh/authorized_keys
grep -qF " $key_body " ~/.ssh/authorized_keys || printf '%s\n' "$line" >>~/.ssh/authorized_keys

# Check what the plan promises rather than assuming it.
if sudo -l -U "$account" 2>/dev/null | grep -q 'may run'; then
  echo "$account must not have sudo" >&2
  exit 1
fi
if sudo -u "$account" test -w /var/run/docker.sock; then
  echo "$account must not reach Docker" >&2
  exit 1
fi
if sudo -u "$account" test -r /var/lib/browseros-remote-builds; then
  echo "$account must not read the pool state" >&2
  exit 1
fi
echo "runner host ready: $account, actions runner $version, dispatcher key restricted to $coordinator"
