# Remote heavy jobs

**Selected source is uploaded verbatim, without redaction. A token written inline
in an ordinary tracked file is sent to the worker.** The exclusions are path based;
the limited private-key header check is not a general secret scanner. Remove inline
secrets before submitting. This is a trusted shared fleet, not a private sandbox.

Existing worktrees can use the lead's published shared launcher with these same action names,
without configuration, copying scripts or updating their branch. The administrator supplies
its installed client/profile; the per-worktree package script below is available after this change lands.

Run from a worktree's `packages/browseros-agent`:

```sh
bun run remote gate
bun run remote image
bun run remote viewer-image
bun run remote spec
bun run remote detector
bun run remote run -- cargo +1.98.0 check -p neo-server --locked
bun run remote status
bun run remote queue
bun run remote fetch-image IMAGE_JOB_UUID
```

New jobs check the coordinator before preparing source. Each status, queue and admission request
first probes SSH with a five-second budget. If it does not answer, the client prints one line
explaining that the usual cause is this machine's outside IPv4 not being admitted. Check that address:

```sh
curl --noproxy '*' -4fsS --connect-timeout 3 --max-time 5 https://api.ipify.org
```

Ask the pool operator to compare it with the SSH allowlist and admit it if needed; retrying
will not help until access is fixed. A timeout can also mean a down VM or a network problem,
so the hint is not proof of an allowlist mismatch. A reachable but full pool still waits for
a slot. The five seconds bound the connection probe, not time in the queue: running jobs,
lease renewals, RPC recovery and artifact transfers keep their existing budgets. `fetch-image`
uses its saved guest endpoint without requiring the coordinator.

`image` builds the SDK's amd64 image on Linux and runs its native acceptance suite.
`viewer-image` uses the same builder for your local Docker architecture, then streams and loads
that image into local Docker. The Rust build cross-compiles with Zig; QEMU runs the arm64
image's setup commands. Because the existing acceptance script deliberately skips
runtime checks for an emulated image, arm64 runtime acceptance runs afterward on the Mac,
under its local heavy/image locks, using the printed command.

`fetch-image` downloads any completed image job separately and verifies the loaded image ID.
Both commands print a viewer command with a unique instance name, an automatic port and
`--image`, so it reuses that image without building or touching the owner's default viewer.
No image is pushed. `spec` preserves the checkout's runner and exit status, using up to
eight workers within that branch's browser cap. Selection flags pass through unchanged;
add `--ci` for the baseline verdict on branches that support it. The experimental `detector`
recipe owns a fresh headless Linux browser and random CDP port, but current-main/control runs
lose CDP in the console-listener phase. **Keep detector runs local** under existing heavy/live
locks until a complete current-main remote run passes; the wrapper preserves its actual 101.

## What the command does

```text
agent in its worktree
  | check coordinator connection (five seconds)
  | capture commit + dirty flag; freeze approved working files
  | ask the shared slot file for the first free VM
  | Crabbox uploads the frozen files, then runs the command
  | stream logs; wait for the guest to confirm completion
  | release the VM immediately
  + save reports locally and remove this job's remote source directory
```

The four VMs stay running. A job reserves one VM only while it syncs and executes. The next
job may come from any developer or worktree. `queue` lists running and waiting jobs by worker
and kind. Each worker may have two jobs total, including waiters; a third is refused with
the two existing job IDs. Gates go before images (including viewer images), then specs,
detectors and other commands, with FIFO within each class. A dead client's waiter is
withdrawn by its independent pipe observer; loss of the Mac expires it after ten seconds.
Running jobs use the separate two-minute cleanup lease described below.

Cargo registry/Git downloads, the public Playwright corpus,
Docker layers and BuildKit caches stay on each host; fresh source directories prevent jobs
from sharing mutable working files. Cargo target and intermediate build directories are also
UUID-private: its mtime freshness check can otherwise reuse a later build while an older
snapshot waits in the queue. Cleanup removes these outputs; compiled Cargo artifacts are
not shared as Cargo-fresh output across jobs. Optional pinned sccache 0.19.0 caches only
registry/Git dependency compilations by content; workspace/path crates always reach rustc
directly. Stable relative Cargo selectors resolve into each job's private directories.
The 20-GiB local disk cache persists, while its foreground server and compiler children
belong to the job's systemd cgroup. Lockfiles and the toolchain partition entries; per-job
`dependency-cache.json` records statistics. Docker's existing dependency-layer cache is
independent. There is no worktree affinity or developer priority.

To compare a suspect gate without compiler caching, use
`bun run remote run -- env REMOTE_DEPENDENCY_CACHE=0 bash crates/neo-sdk/check.sh`.
For administrator rollout, `prepare.sh` installs the cache helpers before publishing
the guest helper. Run `/usr/local/lib/browseros-remote-builds/prepare-cache.sh`, then create
`/etc/browseros-remote-builds/dependency-cache.enabled` after the benchmark passes.
Removing that marker disables caching for new jobs; active jobs continue unchanged.

```text
One agent / one worktree
Agent       Mac snapshot       VM1 slot         VM1
  | freeze ------>|               |               |
  | acquire -------------------->| busy          |
  | Crabbox sync + execute --------------------->|
  |<------------------------------ log + result |
  | release -------------------->| free          |
  |<---------------------------------- reports  |

Two agents at once
Agent A              Shared slots              Agent B
  | acquire A ---------->|<---------- acquire B    |
  |<--------- VM1 busy   |   VM2 busy ------------>|
  | run on VM1          |           run on VM2     |
  | release VM1 ------->|<---------- release VM2   |
  |                     |                          |
  + if all four are busy, the next request waits --+

Warm host versus cold caches (both VMs already running)
Agent                 Warm VM                 Cold-cache VM
  | upload files -------->|                        |
  | run: reuse deps/layers |                        |
  |<----------- result    |                        |
  | upload files -------------------------------->|
  |                 download/compile dependencies  |
  |<------------------------------------ result   |
```

No VM is created or started in a job's path. `pool.py` serializes a small JSON file over SSH
on VM1 using an OS file lock. `guest.py` puts each command in a named systemd service, so it can
stop the whole process tree even if a child detaches. These are short-lived SSH commands,
not a new network service.

The image driver and recipes are installed on every worker, so an older checkout does not
need this feature's files. Explicit source metadata selects that checkout's actual Docker
inputs. A narrow Node preload supplies only a legacy spec report's `git rev-parse HEAD` in
the uploaded agent directory; public corpus Git and other child processes run normally.

### Legacy supervisor compatibility and removal

Current main's Dockerfile owns `supervisor/` and never uses the legacy block in
`build.sh`. It is retained for selected older worktrees whose Dockerfile still
contains `--from=neo-docker`, including the observed `feat/main-sync` at `a22477b21e`
and `feat/neo-sdk-image` at `9ce6bc728`. Their installed remote driver reads only
the five declared supervisor inputs; it does not edit the production directory.
Remove the conditional block and `remote.py:LEGACY_SUPERVISOR` together when all
supported image-source worktrees have rebased to a Dockerfile owning `supervisor/`
and the lead retires support for pre-split frozen snapshots. The selected Dockerfile,
not the installed driver's revision, determines whether compatibility is needed.

## Client setup and three developers

Use the complete **Crabbox 0.83.0** release, including its adjacent runtime directory. Each
developer generates their own SSH key locally. An administrator adds only their **public**
key to the dedicated `buildrunner` user's `authorized_keys` on all four workers and adds their
public IP `/32` to the pilot's SSH rule. Do not share private keys. The worker keys are pinned
in a `known_hosts` file beside each developer's private key, verified through the cloud's
guest-agent channel rather than an unauthenticated first SSH connection.

Put the supplied fleet profile in `~/.config/browseros-remote-builds/config.json`, mode 0600:

```json
{
  "key": "~/.ssh/browseros-remote-builds",
  "crabbox": "/absolute/path/to/pinned/crabbox",
  "coordinator": {"host": "SUPPLIED_PRIVATELY", "user": "buildrunner", "port": 22},
  "runs": "~/.local/state/browseros-remote-builds/runs"
}
```

All developers use the same coordinator. Running many agents or starting a second Mac does
not create another pool. `REMOTE_BUILDS_CONFIG` or `--config PATH` selects another local profile.
The profile contains no Azure credentials; normal jobs need SSH only. Azure IDs, keys,
addresses and profiles do not belong in the repository.

## Source and result boundaries

`remote.py` asks the pinned Crabbox to enumerate Git-tracked and nonignored working files
under `packages/browseros-agent` plus `.github/workflows/test.yml`. Images from older checkouts
also select exactly the five supervisor inputs named in `remote.py:LEGACY_SUPERVISOR` when their
Dockerfile references that named context. Its `EXCLUDE` constant
removes Git metadata, agent state, environment files and credential/key files. Tracked relative file links are materialized only when every target is also explicitly
selected by the manifest; absolute, escaping, excluded, untracked and directory links are
refused. The recognized PEM/OpenSSH-style private-key headers and concurrent edits
are rejected before upload; arbitrary tokens and other key formats can still pass.
Legitimate Rust modules
named `credentials.rs` and `src/target` remain eligible. Only this audited manifest is copied
to a private temporary snapshot on the Mac. A temporary index makes Crabbox preserve those
source paths; it has no commits and is never transferred. Git seeding and overlays are off.

Keep differently named local secrets outside the selected tree, or add their exact paths
to the reviewed launcher's `EXCLUDE` before use and check the resulting manifest.
`.gitignore` can exclude an untracked local file, but does not exclude a tracked file.
No filename rule can remove a token embedded inside otherwise required source; remove
that token from the file. The client must not silently redact inputs and build other bytes.

The guest receives explicit source revision/dirty metadata. The image builder still checks
the actual source digest embedded in the server binary and the binary hash against image
provenance. A revision label alone is not proof of the image's bytes.

No ambient personal, cloud, registry or agent environment is forwarded. Images are keyless.
Each job has a private Docker config and its own UUID. Jobs run as trusted `buildrunner`,
with Docker access on dedicated VMs; they are not isolated from another developer's stored
source or caches. Do not submit untrusted code to this fleet.

Each local `runs/<UUID>/` contains the source manifest and its hashes, `run.log`, `result.json`
and an `artifacts.tar.gz` archive. The independent `guest.log` and service-owned exit record
survive a lost SSH connection. Archives include image acceptance logs and spec/detector reports,
omitting copied upstream tests and dependencies. The client saves the archive without
extracting remote filenames. Artifacts stay remote if downloading fails. Source directories
are deleted only after successful collection; dependency/image caches deliberately remain.

### What a worker retains, and for how long

There is **no time-based deletion or general secret redaction** in these helpers.
Anyone with the trusted worker account/Docker access can read retained data.

| Data on a worker | Lifetime |
| --- | --- |
| Job source and private Cargo output | Removed after successful artifact collection. After cancellation, lost clients or failed collection, kept until recovery collects/cleans that UUID or an operator deletes it. |
| Durable logs, info/exit/cleanup receipts, cancellation markers, per-job Docker config and cached artifact archives under the jobs state directory | No automatic expiry; kept until deliberate operator cleanup or disk deletion. Output can itself contain source, tokens or test data. |
| UUID-tagged images and Docker/BuildKit layers | No repository TTL; kept until explicit image/cache cleanup or engine eviction. Inputs copied into an image/layer can survive source-directory cleanup. |
| Cargo registry/Git downloads and public test corpus | No repository TTL; kept until operator cleanup or disk deletion. |
| sccache dependency entries | Size-evicted within 20 GiB per worker, not time-expired; may remain until eviction, drained cache cleanup or disk deletion. |

Local run logs/manifests/archives also have no automatic TTL. Plan explicit retention
with the operator; deleting the source folder is not proof that all copies are gone.
Drain first and use job receipts/UUIDs for cleanup, never a broad delete on a busy worker.

## Interrupted jobs

The client tests SSH before dispatch and skips a failed host for two minutes. New jobs renew
a two-minute lease every twenty seconds. An independent coordinator timer checks every fifteen
seconds, fences expired UUIDs, stops their recorded service and Docker work, then releases the
slot only after a matching durable safe receipt. Expiry alone never makes a host free; an
unreachable guest stays unavailable. The timer uses an internal forced-command SSH key that
can only cancel jobs and never opens a shell. It does not stop a VM. The timer/key/profile
must be provisioned separately using [COORDINATOR.md](COORDINATOR.md); worker preparation
alone provides no independent reaping. Automatic expiry retains remote logs/source for
collection with `recover`; it cannot write results to an absent Mac. To recover any uncertain
job manually:

```sh
bun run remote recover JOB_UUID
```

Recovery fences late starts, stops the named job's systemd service, and proves it inactive
before releasing its slot. It collects results/logs and removes that job's source, and can
resume collection even after a previous attempt released the slot. Terminal receipts make
duplicate recovery safe while another job uses the host. A host lock and durable owner also
prevent an orphaned service from overlapping another UUID. Docker runs outside that service's cgroup, so cancellation also
restarts Docker on this exclusively reserved worker and removes only resource IDs absent
from the persisted pre-job baseline. The fleet must not host independent running containers;
a worker with any at admission rejects the job, and an unavailable Docker inventory fails
admission. Docker's built-in networks are excluded from job cleanup. Never delete a pool record just to unblock
the queue. If the guest cannot prove cleanup, repair that task-owned VM before recovery.

### Hard wall limits, separate from client-loss leases

Every reservation also has a coordinator-owned `wallUntil`, frozen when it takes a
slot. Queue wait is excluded; sync/execution are included. A live client cannot
extend it by heartbeat, retry or changing its displayed kind. Defaults are deliberately
generous compared with the measured runs:

| Kind | Measured service time | Hard wall limit |
| --- | --- | --- |
| Gate | About 5 minutes | 20 minutes |
| Image / viewer image | About 8 / 16 minutes | 45 minutes |
| Spec | About 13 minutes for full baseline plus build | 45 minutes |
| Detector (experimental; currently kept local) | About 2 minutes | 15 minutes |
| Other explicit `run` | Workload dependent | 30 minutes |

The coordinator fences renewals at the deadline. Its independent reaper then uses
the same owned-process/Docker cancellation and matching safe receipt as a lost-client
lease; it never frees an unproven host. Timer scheduling and cleanup add latency,
and an unreachable guest stays reserved until cleanup is proven. The coordinator/timer
must remain available; this is not a guest-only timer that misses Docker work.

For deliberately longer work, split it or have the operator set finite per-class
`policy.wallSeconds` values in `pool.json` under its sidecar lock **before** admission.
For example, `{"policy":{"wallSeconds":{"run":3600}}}` raises future custom runs to
one hour; merge that field without replacing hosts or live job records. Values must
be integer seconds from 1 through 86400; there is no unlimited override or client flag.
Policy changes do not extend admitted jobs. Older reservations without `wallUntil`
use their original `started` time and class limit; drain before upgrading coordinator
helpers so existing work is not unexpectedly cancelled by the new policy.

## Administrator setup

Create the approved, dedicated VMs separately from jobs. Cloud-init installs Rust 1.98.0,
clippy/rustfmt and complete Crabbox 0.83.0. Transfer the same audited source manifest, then run
`bash tools/remote-builds/prepare.sh` on each worker. It installs pinned Docker/buildx,
Node 26.13.0, Bun 1.4.2, registered arm64 QEMU and the browser/fonts from `browser.lock`;
reconnect SSH afterward.

**This prepares workers only.** After initializing the fresh pool below, complete
the key, pinned host identities, reaper profile, systemd units and readiness probes in
[COORDINATOR.md](COORDINATOR.md) on the coordinator. It is a required separate deployment
step. Do not declare a fleet ready just because `prepare.sh` or an SSH job succeeded.

On VM1 initialize `/var/lib/browseros-remote-builds/pool.json` once, owned by `buildrunner`:

```json
{"hosts":[
  {"name":"rb-build-01","host":"SUPPLIED_PRIVATELY","user":"buildrunner","port":22,"job":null},
  {"name":"rb-build-02","host":"SUPPLIED_PRIVATELY","user":"buildrunner","port":22,"job":null},
  {"name":"rb-build-03","host":"SUPPLIED_PRIVATELY","user":"buildrunner","port":22,"job":null},
  {"name":"rb-build-04","host":"SUPPLIED_PRIVATELY","user":"buildrunner","port":22,"job":null}
]}
```

Do not overwrite an existing pool file or clear reservations when upgrading helpers. The
coordinator is a single availability dependency; if it is unreachable, clients fail closed.
Use the pilot's external `STATE.md` for current inventory, costs and Azure stop/delete commands.
Nothing in the client powers VMs off, changes cloud resources or removes the fleet.

Run focused checks with:

```sh
TEST_CRABBOX=/path/to/0.83.0/crabbox python3 -m unittest discover -s tools/remote-builds -p 'test_*.py' -v
```

The existing `Tests / claw-sdk-quality` CI job runs these Python tests without fleet
access. Transfer fixtures are explicitly skipped there because that job does not
install Crabbox; the command above with the **complete** pinned release runs them too.
The tests use local temporary state and an owned synthetic process; they never modify
the real coordinator, call cloud APIs, start Docker or submit a shared-pool job.

Local Mac jobs still obey the lead's heavy/image/spec locks. Proposed worker-rule changes:
use `bun run remote gate|image|spec` for supported Linux steps, with the shared remote
slot replacing the local heavy/image/spec locks for those commands only. Keep Mac app/UI,
macOS Rust targets, detector runs, native arm64 runtime acceptance and Chromium work on the Mac with their existing gates. The lead applies its
own `WORKER.md` and launcher changes; this implementation does not edit them.
