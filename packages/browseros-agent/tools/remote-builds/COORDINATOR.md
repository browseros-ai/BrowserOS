# Required coordinator setup

`prepare.sh` installs worker tools/helpers only. It does **not** provision the
reaper key, configuration or timer. Complete this separate procedure once on the
coordinator, and verify it before allowing any clients to submit jobs. Commands
run as `buildrunner`; this trusted account needs `sudo -n true` to succeed on each
dedicated worker for systemd and Docker cancellation. Supply private endpoints
and verified host public keys through your operator channel, never this repository.

For an upgrade, stop new submissions and drain/recover existing jobs first. Keep
the existing pool file, owner/job records and authorized keys. Install helper code
atomically, without rerunning the full Docker/package preparation on busy workers.
The new coordinator derives any legacy wall deadline from its original start;
an already over-budget reservation can immediately need cancellation on upgrade.

## 1. Pool and private cancellation identity

Initialize the README's `pool.json` only on a fresh fleet. Its host names must
match the reaper mapping below. Ensure the existing state directories are private:

```sh
sudo install -d -m 0700 -o buildrunner -g buildrunner /var/lib/browseros-remote-builds
sudo install -d -m 0700 -o buildrunner -g buildrunner /var/lib/browseros-remote-builds/jobs
sudo -n true
umask 077
if [ ! -f /var/lib/browseros-remote-builds/coordinator_ed25519 ]; then
  ssh-keygen -q -t ed25519 -N '' -C remote-builds-lease-reaper -f /var/lib/browseros-remote-builds/coordinator_ed25519
fi
cat /var/lib/browseros-remote-builds/coordinator_ed25519.pub
```

The private key stays on the coordinator, mode 0600. Copy **only its public key**
to every worker, including the coordinator itself. On each worker, append one
entry to `~/.ssh/authorized_keys` as follows, substituting the coordinator's private
source address and its public key. Preserve existing entries and do not append a
duplicate key:

```text
from="SUPPLIED_COORDINATOR_PRIVATE_ADDRESS",restrict,command="/usr/bin/python3 /usr/local/lib/browseros-remote-builds/guest.py lease-rpc" ssh-ed25519 SUPPLIED_PUBLIC_KEY remote-builds-lease-reaper
```

Keep `~/.ssh` mode 0700 and `authorized_keys` mode 0600, owned by `buildrunner`.
Permit TCP/22 between the coordinator's private address and workers. Do not add a
public ingress rule for this key. `restrict` disables forwarding/PTY/user-rc
facilities; the forced guest command additionally accepts only original command
`lease-cancel`, with a validated job UUID on stdin.

## 2. Pinned host identities and reaper profile

Obtain each worker's SSH host public key through the authenticated cloud
guest-agent/operator channel. Do not trust an unauthenticated `ssh-keyscan` result.
Write `/var/lib/browseros-remote-builds/reaper-known-hosts` on the coordinator,
mapping the private address used below to that verified key:

```text
SUPPLIED_PRIVATE_ENDPOINT_01 ssh-ed25519 SUPPLIED_VERIFIED_HOST_PUBLIC_KEY_01
SUPPLIED_PRIVATE_ENDPOINT_02 ssh-ed25519 SUPPLIED_VERIFIED_HOST_PUBLIC_KEY_02
SUPPLIED_PRIVATE_ENDPOINT_03 ssh-ed25519 SUPPLIED_VERIFIED_HOST_PUBLIC_KEY_03
SUPPLIED_PRIVATE_ENDPOINT_04 ssh-ed25519 SUPPLIED_VERIFIED_HOST_PUBLIC_KEY_04
```

Write `/var/lib/browseros-remote-builds/reaper.json` separately from the client
profile. The four mapping keys must exactly match `pool.json` host names; values
are worker private addresses/DNS endpoints reached on port 22 as `buildrunner`:

```json
{
  "key": "/var/lib/browseros-remote-builds/coordinator_ed25519",
  "knownHosts": "/var/lib/browseros-remote-builds/reaper-known-hosts",
  "hosts": {
    "rb-build-01": "SUPPLIED_PRIVATE_ENDPOINT_01",
    "rb-build-02": "SUPPLIED_PRIVATE_ENDPOINT_02",
    "rb-build-03": "SUPPLIED_PRIVATE_ENDPOINT_03",
    "rb-build-04": "SUPPLIED_PRIVATE_ENDPOINT_04"
  }
}
```

Both files are owned by `buildrunner`, mode 0600. Verify matching names and the
restricted identity without cancelling any real job. This probe deliberately
sends an invalid UUID, which fails before a cancellation marker or service exists:

```sh
chmod 0600 /var/lib/browseros-remote-builds/pool.json /var/lib/browseros-remote-builds/reaper.json /var/lib/browseros-remote-builds/reaper-known-hosts /var/lib/browseros-remote-builds/coordinator_ed25519
python3 - <<'PY'
import json, subprocess
from pathlib import Path
root = Path('/var/lib/browseros-remote-builds')
profile = json.loads((root / 'reaper.json').read_text())
pool = json.loads((root / 'pool.json').read_text())
assert set(profile['hosts']) == {host['name'] for host in pool['hosts']}
for name, endpoint in profile['hosts'].items():
    ssh = ['ssh', '-i', profile['key'], '-o', 'BatchMode=yes',
           '-o', 'IdentitiesOnly=yes', '-o', 'ForwardAgent=no',
           '-o', 'StrictHostKeyChecking=yes', '-o', 'ConnectTimeout=10',
           '-o', 'UserKnownHostsFile=' + profile['knownHosts'],
           'buildrunner@' + endpoint]
    denied = subprocess.run(ssh + ['true'], capture_output=True, text=True, timeout=20)
    assert denied.returncode != 0 and 'restricted coordinator command' in denied.stderr, name
    probe = subprocess.run(ssh + ['lease-cancel'], input='{"job":"setup-probe"}',
                           capture_output=True, text=True, timeout=20)
    assert probe.returncode != 0 and 'invalid job identifier' in probe.stderr, name
    print(name + ': pinned identity and cancellation-only command verified')
PY
```

## 3. Install and verify the independent timer

From the audited `packages/browseros-agent` source directory on the coordinator:

```sh
sudo install -m 0644 tools/remote-builds/browseros-remote-builds-reaper.service /etc/systemd/system/browseros-remote-builds-reaper.service
sudo install -m 0644 tools/remote-builds/browseros-remote-builds-reaper.timer /etc/systemd/system/browseros-remote-builds-reaper.timer
sudo systemctl daemon-reload
sudo systemctl enable --now browseros-remote-builds-reaper.timer
sudo systemctl start browseros-remote-builds-reaper.service
systemctl is-enabled browseros-remote-builds-reaper.timer
systemctl is-active browseros-remote-builds-reaper.timer
systemctl show browseros-remote-builds-reaper.service -p Result -p ExecMainStatus
journalctl -u browseros-remote-builds-reaper.service -n 30 --no-pager
```

Require `enabled`, `active`, `Result=success`, `ExecMainStatus=0` and no profile,
key, host-identity or cleanup errors. A fresh empty pool permits this first sweep
without touching a workload. Timer active alone is insufficient if its service
fails. Do not declare the pool ready until both the restricted SSH probes and the
service/timer checks pass.

The timer waits 15 seconds after each sweep completes; cleanup attempts are bounded
and serialized. At a deadline, renewal is fenced and the reaper requests normal
owned-service/Docker cancellation. An unreachable worker stays reserved pending
proof; no timer can guarantee immediate process death over a broken connection.
Investigate `retained pending cleanup` and use normal recovery, not a cleared row.

To disable monitoring deliberately, drain the pool, then use
`sudo systemctl disable --now browseros-remote-builds-reaper.timer`. This neither
deallocates VMs nor cancels live jobs by itself. No automatic record/cache retention
timer is provisioned by this procedure.

## GitHub runners

Optional. This lets members run jobs through the **Pool run** workflow (README, "Run a job from
GitHub"). It adds a dispatcher service on the coordinator and a runner account on each runner
host. The SSH clients, the reaper and the existing records are unchanged. Keep the coordinator
itself out of the runner hosts: it holds the pool state and every key below, and member code
should never run beside them.

### Organisation side (an organisation owner)

1. Create an organisation-owned GitHub App with these permissions:
   - Organisation: **Self-hosted runners**, read and write.
   - Repository: **Actions**, read; **Metadata**, read.

   It needs no webhook. Install it on this repository only. Move its private key from the
   downloaded file straight to the coordinator, as below. Never paste it into a terminal,
   commit it, or put it in a repository.
2. Create the runner group with the App's token. Every field matters:
   - `visibility: selected`, listing only this repository.
   - `allows_public_repositories: true`. A public repository needs it; the default admits only
     private ones.
   - `restricted_to_workflows: true`, with `selected_workflows` naming only
     `browseros-ai/BrowserOS/.github/workflows/pool-run.yml@refs/heads/main`.

   A fork's workflow can then never be given these runners.

### Coordinator

First install the current `pool.py` the usual way, at a natural idle (0 running, 0 waiting),
keeping a backup. An older `pool.py` ignores the dispatcher's host list. The dispatcher notices,
hands that machine straight back and halts, but it can do nothing useful until the upgrade.

As `buildrunner`, with the same private directory as the reaper:

```sh
umask 077
ssh-keygen -q -t ed25519 -N '' -C pool-runner-dispatcher -f /var/lib/browseros-remote-builds/runners_ed25519
```

From the machine that downloaded the App's key, file to file, so the key is never shown:

```sh
ssh COORDINATOR 'umask 077; cat > /var/lib/browseros-remote-builds/github-app.pem' < DOWNLOADED_KEY.pem
rm DOWNLOADED_KEY.pem
```

`/var/lib/browseros-remote-builds/runners.json` (mode 0600) holds ids and paths only:

```json
{
  "org": "browseros-ai",
  "repository": "browseros-ai/BrowserOS",
  "workflow": ".github/workflows/pool-run.yml",
  "branches": ["main"],
  "label": "browseros-pool",
  "runnerGroupId": 0,
  "appId": 0,
  "installationId": 0,
  "appKey": "/var/lib/browseros-remote-builds/github-app.pem",
  "key": "/var/lib/browseros-remote-builds/runners_ed25519",
  "hosts": ["SUPPLIED_RUNNER_HOST_NAMES"]
}
```

`hosts` lists the `pool.json` names that may serve GitHub jobs. The dispatcher passes it with
each `acquire`, and `pool.py` gives that job only those hosts. Private endpoints and pinned
host keys come from `reaper.json` and `reaper-known-hosts`.

Before minting any runner, and once a minute after that, the dispatcher reads the runner group
back. It mints nothing unless the group admits only this repository and only the workflow refs
in `runners.json`. Its log says why when it refuses.

Install the dispatcher and start it:

```sh
sudo install -m 0644 tools/remote-builds/runners.py /usr/local/lib/browseros-remote-builds/runners.py
sudo install -m 0644 tools/remote-builds/browseros-remote-builds-runners.service /etc/systemd/system/
sudo systemctl daemon-reload
sudo systemctl enable --now browseros-remote-builds-runners.service
journalctl -u browseros-remote-builds-runners.service -n 20 --no-pager
```

Restart it after installing a new `pool.py`. A long-running process keeps the module it
loaded at start.

### Each runner host

Do this while `bun run remote status` shows the host free. From the same audited source copy
used for `prepare.sh`:

```sh
bash tools/remote-builds/prepare-runner.sh runners_ed25519.pub SUPPLIED_COORDINATOR_PRIVATE_ADDRESS
```

It does the following, and checks the result:
- Creates `poolrunner`: no password, no login shell, no sudo, no other group.
- Denies it cron.
- Unpacks the Actions runner into `/opt/actions-runner`, checked against its published sha256.
- Installs `runner.sh`, `runner-hook.sh` and `guest.py`.
- Makes `/work/remote-builds` traversable but not listable.
- Adds the dispatcher's key, restricted to `guest.py runner-rpc` and to the coordinator's
  address.

### Removing it

1. Stop and disable `browseros-remote-builds-runners.service`. Jobs it was serving are cancelled
   by the reaper once their lease lapses.
2. Delete the `browseros-pool-*` runners, the runner group and the App.
3. On each runner host, while idle:
   - Remove the `pool-runner-dispatcher` line from `authorized_keys`.
   - Remove the `poolrunner` line from `/etc/cron.deny`.
   - `sudo userdel poolrunner`.
   - `sudo rm -rf /opt/actions-runner /work/pool-runner`.
   - Remove `runner.sh` and `runner-hook.sh`.
   - `sudo chmod 0755 /work/remote-builds`.
4. Remove `runners.json`, `github-app.pem` and `runners_ed25519` from the coordinator.
