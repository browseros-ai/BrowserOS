#!/usr/bin/env bash
# One GitHub Actions job on this pool machine, as the unprivileged `poolrunner` account.
#
# guest.py runs this as the payload of an ordinary pool job (see `runner-rpc`), so the
# pool slot, its lease and wall, and systemd's cgroup kill bound everything started here.
# The runner gets a one-job (just-in-time) registration, a fresh home, runner copy and
# work directory, and no sudo, Docker or pool access. Members' workflow code runs only
# under that account.
#
# The account exists only for these jobs, and this job owns the whole machine (one pool
# reservation per host). So anything `poolrunner` owns when a job starts or ends is a
# leftover from a runner job, and is removed. That keeps one job from leaving anything
# for the next.
#
# `runner.sh --sweep` only removes and checks. guest.py runs it after the job's service has
# stopped and before it records the host as safe to release, so a job cancelled before
# its own exit sweep still leaves nothing behind.
set -euo pipefail
base=/work/pool-runner
account=poolrunner

sweep() {
  # Kill by account, not by name: no other job runs on this machine now, and only
  # runner jobs run as this account. Systemd has already killed the unit's own cgroup.
  sudo -n pkill -KILL -u "$account" 2>/dev/null || true
  for shared in /tmp /var/tmp /var/crash /dev/shm /run/lock; do
    sudo -n find "$shared" -xdev -mindepth 1 -user "$account" -delete 2>/dev/null || true
  done
  # System V shared memory, semaphores and queues outlive the processes that made them.
  # Their creator can always remove them, so this runs as the account itself.
  sudo -n -u "$account" python3 - <<'PY' || true
import os, subprocess
uid = str(os.getuid())
for kind, flag in (("shm", "-m"), ("sem", "-s"), ("msg", "-q")):
    rows = open(f"/proc/sysvipc/{kind}").read().splitlines()
    header = rows[0].split()
    for fields in (row.split() for row in rows[1:]):
        if uid in (fields[header.index("uid")], fields[header.index("cuid")]):
            subprocess.run(["ipcrm", flag, fields[1]], check=False)
PY
  sudo -n find "$base" -mindepth 1 -maxdepth 1 -exec rm -rf --one-file-system -- {} + || true
}

swept() {
  ! pgrep -u "$account" >/dev/null && [ -z "$(sudo -n find "$base" -mindepth 1 -maxdepth 1 -print -quit)" ]
}

sweep
if [ "${1:-}" = --sweep ]; then
  swept || { echo 'runner account processes or files could not be removed' >&2; exit 1; }
  exit 0
fi
# Refuse to start on top of anything the sweep could not remove.
swept || { echo 'a previous runner job left processes or files that could not be removed' >&2; exit 1; }

job=${REMOTE_BUILD_JOB:?guest supervisor must supply the job}
[[ $job =~ ^[0-9a-f]{32}$ ]] || { echo 'invalid job identifier' >&2; exit 2; }
request=/var/lib/browseros-remote-builds/jobs/$job.runner.json
dir=$base/$job
dist=/opt/actions-runner/current
hook=/usr/local/lib/browseros-remote-builds/runner-hook.sh
trap 'sweep' EXIT
[ -f "$request" ] || { echo 'no runner registration for this job' >&2; exit 2; }

sudo -n install -d -m 0700 -o "$account" -g "$account" "$dir" "$dir/home" "$dir/tmp" "$dir/runner"
sudo -n -u "$account" cp -a --no-preserve=ownership "$dist/." "$dir/runner/"
# Hand the registration over in a file only the runner account can read, never on a
# command line (visible in /proc) or in the repository. guest.py's copy goes at once.
python3 -c 'import json, sys; sys.stdout.write(json.load(open(sys.argv[1]))["jitconfig"])' "$request" \
  | sudo -n -u "$account" sh -c 'umask 077; cat > "$1"' _ "$dir/jit"
repository=$(python3 -c 'import json, sys; print(json.load(open(sys.argv[1]))["repository"])' "$request")
workflows=$(python3 -c 'import json, sys; print(" ".join(json.load(open(sys.argv[1]))["workflows"]))' "$request")
rm -f -- "$request"

echo "Starting a one-job runner as $account"
# env -i: nothing of this payload's environment (its HOME, Cargo and cache settings)
# reaches the job. The runner reads its registration from ACTIONS_RUNNER_INPUT_JITCONFIG
# and removes that variable from its own environment before any job starts.
status=0
sudo -n -u "$account" -- /usr/bin/env -i \
  HOME="$dir/home" TMPDIR="$dir/tmp" LANG=C.UTF-8 \
  PATH=/usr/local/bin:/usr/bin:/bin \
  ACTIONS_RUNNER_HOOK_JOB_STARTED="$hook" \
  POOL_RUNNER_REPOSITORY="$repository" POOL_RUNNER_WORKFLOWS="$workflows" \
  /bin/bash -c 'umask 077
    ACTIONS_RUNNER_INPUT_JITCONFIG=$(cat "$1")
    rm -f -- "$1"
    export ACTIONS_RUNNER_INPUT_JITCONFIG
    cd "$2" && exec ./run.sh' _ "$dir/jit" "$dir/runner" || status=$?
echo "Runner exited with status $status"
exit "$status"
