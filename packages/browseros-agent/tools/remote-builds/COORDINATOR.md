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
