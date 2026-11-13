# Remote heavy jobs

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
bun run remote fetch-image IMAGE_JOB_UUID
```

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
add `--ci` for the baseline verdict on branches that support it. `detector` starts its own
headless Linux browser with a fresh profile and random CDP port.

## What the command does

```text
agent in its worktree
  | capture commit + dirty flag; freeze approved working files
  | ask the shared slot file for the first free VM
  | Crabbox uploads the frozen files, then runs the command
  | stream logs; wait for the guest to confirm completion
  | release the VM immediately
  + save reports locally and remove this job's remote source directory
```

The four VMs stay running. A job reserves one VM only while it syncs and executes. The next
job may come from any developer or worktree. Cargo registry/Git downloads, the public Playwright corpus,
Docker layers and BuildKit caches stay on each host; fresh source directories prevent jobs
from sharing mutable working files. Cargo target and intermediate build directories are also
UUID-private: its mtime freshness check can otherwise reuse a later build while an older
snapshot waits in the queue. Cleanup removes these outputs; compiled Cargo artifacts are
not shared across jobs. There is no worktree affinity or developer priority.

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
refused. Private-key blocks and concurrent edits are rejected before upload. Legitimate Rust modules
named `credentials.rs` and `src/target` remain eligible. Only this audited manifest is copied
to a private temporary snapshot on the Mac. A temporary index makes Crabbox preserve those
source paths; it has no commits and is never transferred. Git seeding and overlays are off.

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

## Interrupted jobs

The client tests SSH before dispatch and skips a failed host for two minutes. New jobs renew
a two-minute lease every twenty seconds. An independent coordinator timer checks every fifteen
seconds, fences expired UUIDs, stops their recorded service and Docker work, then releases the
slot only after a matching durable safe receipt. Expiry alone never makes a host free; an
unreachable guest stays unavailable. The timer uses an internal forced-command SSH key that
can only cancel jobs and never opens a shell. It does not stop a VM. Reservations created by
older clients remain explicit-recovery jobs. Automatic expiry retains remote logs/source for
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

## Administrator setup

Create the approved, dedicated VMs separately from jobs. Cloud-init installs Rust 1.98.0,
clippy/rustfmt and complete Crabbox 0.83.0. Transfer the same audited source manifest, then run
`bash tools/remote-builds/prepare.sh` on each worker. It installs pinned Docker/buildx,
Node 26.13.0, Bun 1.4.2, registered arm64 QEMU and the browser/fonts from `browser.lock`;
reconnect SSH afterward.

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

Local Mac jobs still obey the lead's heavy/image/spec locks. Proposed worker-rule changes:
use `bun run remote gate|image|spec|detector` for supported Linux steps, with the shared remote
slot replacing the local heavy/image/spec locks for those commands only. Keep Mac app/UI,
macOS Rust targets, native arm64 runtime acceptance and Chromium work on the Mac with their existing gates. The lead applies its
own `WORKER.md` and launcher changes; this implementation does not edit them.
