# The Neo SDK image

A Linux image of the BrowserOS neo browser and this commit's SDK server, `browseros-claw-sdk`. Agents get
one MCP tool, `run` (Playwright JavaScript against the browser), and the host gets the `/v1` host API on a
Unix socket. A host can build on it with `FROM neo-sdk:<tag>` or run it on its own.
Production uses the default Neo image
(`packages/browseros/docker/neo`, claw-server), which this one shares no file with. CI builds and tests
it in `.github/workflows/neo-sdk-image.yml`, which publishes it only when asked (Publish below).

```
build.sh ──buildx──▶ Dockerfile ─┬─ server   (build machine's arch): this workspace ─▶ browseros-claw-sdk
                                 └─ neo-sdk  (target arch): ubuntu + browser.lock's .deb + the server
                                              + supervisor/ (this folder)
accept.sh <image> ──▶ disposable containers ──▶ PASS / FAIL / SKIP per check
```

The browser supervisor (`supervisor/`: `neo-entrypoint`, the healthcheck, `neo-quit`, `neo-server-info`,
openbox's settings) is this image's own. It started as the default image's and still runs either server,
choosing from the server binary's version marker; a change to it changes this image only.
`supervisor/entrypoint_test.py` tests it (`python3 supervisor/entrypoint_test.py`).

## Build it

```sh
crates/neo-sdk/neo-server/docker/build.sh --platform linux/arm64   # a dev Mac
crates/neo-sdk/neo-server/docker/build.sh --platform linux/amd64   # host containers
```

The tag is `neo-sdk:<browser>-<commit>-<arch>`, with `-dirty` before the arch when
`packages/browseros-agent` (which holds every input) has uncommitted changes. `build.sh` prints `TAG=`, `IMAGE_ID=` and the
image's `provenance.json`. The server is cross-compiled with zig, so an amd64 image builds on an Apple
silicon Mac without emulating rustc. The first build takes about 7 minutes; Cargo's cache is kept per
architecture after that, shared by every checkout built on the Docker daemon.

That cache never decides which server an image gets. Cargo judges freshness by file mtimes, so a
checkout whose sources were older than the cache's last build once shipped that build's server under
its own revision. `build-server.sh` (the Dockerfile's server step) therefore cleans the workspace's own
crates from the cache before each build, keeping the registry and the third-party crates' output, and
fails unless Cargo compiled every workspace crate in that step and the binary carries the digest of the
sources the step copied. `build.sh` then copies the server out of the new image and tags the image only
if that binary's source digest, sha256 and the revision match `provenance.json`; otherwise it exits 1
and leaves the image untagged. `NEO_SDK_BUILD_JOBS=N` caps Cargo's parallel jobs on a busy machine.

## Check it

```sh
crates/neo-sdk/neo-server/docker/accept.sh "$TAG"
```

It boots the image in throwaway containers on a private `--internal` network, drives a real page through
`run`, and checks the host contract below, the stop, persistence, a browser restart and a kill of every
process at once, all under only the capabilities listed below. The header of `accept.sh` lists each check.
`pdf-downloads` checks the host's exact PDF attachment, four valid PDF cases (attachment, inline viewer,
same-origin download attribute, and typed Blob), and a ZIP attachment. It compares the browser file,
the `saveAs()` copy, and the host endpoint's bytes by SHA-256. Inline PDFs open in Chromium's viewer
without a download event.
`tab-keep` polls the host tab listing for idle cleanup with three-minute deadlines, then checks
that kept tabs remain open. After release it checks that tabs stay open for a fresh idle interval
and polls for their eventual closure. These deadlines allow delayed sweeps while still failing
when cleanup never happens.
An image for another architecture than the Docker engine's gets the static `identity` check only, because
booting it would be emulated. CI runs both scripts natively on amd64 and arm64
(`.github/workflows/neo-sdk-image.yml`) on every pull request and push that changes the image's inputs,
with the supervisor's and the scripts' own tests; the accept logs and `provenance.json` are its artifacts.

## Publish

Only `neo-sdk-image.yml` publishes this image, and only to `ghcr.io/browseros-ai/neo-sdk`, never to
production's `ghcr.io/browseros-ai/neo`. A pull request or a push (to `main` too) builds and tests only,
keyless. It publishes the two images `accept.sh` passed, unchanged, when asked from `main`:

```sh
gh workflow run neo-sdk-image.yml --ref main -f publish=true -f confirm=neo-sdk   # :<version>
```

A published image carries both CAPTCHA provider keys ([CAPTCHA keys](#captcha-keys)). They come from
the environment `neo-captcha`, which admits protected branches only, so the run refuses `publish=true`
on any other ref before it builds anything, and a tag does not publish. It also fails before the build
when either key is empty, and before the push when the image does not carry both. A publishing run has
a concurrency group of its own, so a push to `main` neither cancels it nor replaces it while it waits.

`<version>` is `build.sh`'s, `<browser>-<9-char commit>`; each architecture is also at `:<version>-amd64`
and `:<version>-arm64`.

## Develop with it

`dev.sh` runs this checkout's image with the dev kit (`devkit/`, walked through in `devkit/HOWTO.md`): a
viewer and a trace UI over the host API, and `dev.sh run` to send scripts. `smoke_sdk.py` and
`check_display.py` check a running container (`dev.sh smoke`).

## The contract

| Input | Default | Meaning |
|---|---|---|
| `/data` (mount) | none | Persistent state, below. One container per `/data`. |
| `PUID`, `PGID` | 1000 | The uid and gid Neo runs as. |
| `NEO_HOST_GID` | neo's gid | The group allowed to connect to `/run/neo/host.sock`. |
| `CLAW_HOST_TOKEN_FILE` | unset | The host API's bearer token, 32 or more visible ASCII bytes, readable by Neo's uid. Unset: no host API; `run` still works. |
| `CLAW_HOST_SOCKET` | `/run/neo/host.sock` | Moves the socket. Keep it off a Docker Desktop file share, which cannot hold a socket. |
| `CLAW_HOST_TCP` | unset | Development only: also serves the host API on this TCP address, token only. |
| `NEO_MCP_REMOTE` | `false` | `false`: the browser's MCP ports refuse every peer but loopback. `true` is needed to publish 9010, and admits every peer that can route to the container. |
| `BROWSEROS_CAPSOLVER_API_KEY_FILE`, `BROWSEROS_2CAPTCHA_API_KEY_FILE` | `/home/neo/.config/browseros/captcha/capsolver-api-key`, `…/2captcha-api-key` | CAPTCHA provider keys, read if the files exist. The published image carries both; a local or pull request build none ([CAPTCHA keys](#captcha-keys)). |
| `NEO_SCREEN`, `NEO_SANDBOX`, `NEO_SOFTWARE_GRAPHICS`, `NEO_DISABLE_BROWSEROS_EXTENSIONS`, `NEO_ARGS`, `NEO_DOWNLOAD_DIR`, `TZ` | as the Neo image | The same meaning as in [the Neo image's README](../../../../../browseros/docker/neo/README.md). The browser always runs with `--disable-back-forward-cache`, as Playwright launches Chromium; `NEO_ARGS` adds to it. |
| `NEO_CLEAR_SESSIONS` | `false` | `true`: before every start of the browser, the entrypoint removes the profile's saved sessions, `/data/profile/Default/Sessions`, where values typed into pages rest ([Saved sessions](#saved-sessions)). |
| `BROWSERCLAW_HOSTED_MODE` | `1` | Must stay `1`. It also starts the browser with `--disable-browseros-metrics` ([Product analytics](#product-analytics)). |

| Output | Where |
|---|---|
| Agent MCP (`run`), `/system/health`, `/system/ready` | `http://127.0.0.1:9010/mcp`, and the same over TLS (self-signed) on `https://127.0.0.1:9011/mcp`. No auth on either. |
| Host API `/v1`: runs, operations and their screenshots, each run's downloads (`/v1/runs/{runId}/downloads/{index}`) and the bytes too large for its reply (`/v1/runs/{runId}/blobs/{blobId}`), the trace, events, the live view (`/v1/tabs/{tabId}/screencast`, `/video`), the VNC display, locks, fits, takeovers and the settings (`/v1/settings`) ([openapi.yaml](../../neo-sdk-api/openapi.yaml)) | `/run/neo/host.sock`, mode 0660, group `NEO_HOST_GID`, plus `Authorization: Bearer <token>` |
| Health | the image's `HEALTHCHECK`: the entrypoint finished and `/system/ready` answers through 9010 |
| Provenance | `/usr/share/neo-sdk/provenance.json` (commit, the digest of the server's build sources, browser, the server's path, sha256, version and the source digest compiled into it, toolchain), `/usr/share/neo-sdk/browser.lock`, and the OCI `revision` and `version` labels |

### File reads and protected files

The image exports `CLAW_DOWNLOADS_DIR` from its downloads directory and `CLAW_PROFILE_DIR`
from its browser profile. `GET /v1/settings` reports that downloads folder as the default
`files.readDirs` list. A downloaded path and a copy saved under that folder can be uploaded
without changing settings. Owner files can be sent as byte payloads.

To read another folder by path, the host adds it with `PATCH /v1/settings`, such as
`{"files":{"readDirs":["/data/downloads","/etc"]}}`. The list accepts at most 16 absolute
folders. Empty denies path reads; nonexistent folders grant nothing. Symlinks resolve before
component containment is checked. Changes apply to the next file use in an existing run and
survive a server restart. `PUT {}` restores the downloads default.

Even `["/"]` cannot allow Neo's state directory, browser profile, host token or CAPTCHA key
files. Scripts cannot overwrite those locations with a download or screenshot either.

The same goes for Neo's home, `/home/neo`, the `HOME` of the server, the browser, its crash
reporter, Xvnc and the window manager. They read files there without being asked: the
browser's certificate store `~/.pki/nssdb`, fontconfig's files, and the crash reporter's
settings. A folder the host lists in `files.readDirs` inside the home stays usable, unless its
first name below the home starts with a dot. A save into it may make it and the missing folders
above it inside the home, and no other folder there. `/home/neo/Downloads` links to
`/data/downloads`, and a path through it is judged by that real path. If an image runs Neo with
`HOME=/` (a user with no passwd entry), every write outside the downloads folder and the listed
folders is refused.

Directory uploads that contain a protected location are refused. Writes elsewhere retain
Playwright's existing behavior, including `download.saveAs()` outside the read roots.

### Ports

| Port | What | Auth | Reachable by |
|---|---|---|---|
| 9010 | the browser's MCP proxy, HTTP, `0.0.0.0` | none | every process in the container's network namespace; with `NEO_MCP_REMOTE=true`, every peer that can route to the container |
| 9011 | the same proxy over TLS, `0.0.0.0`. It cannot be turned off and is not in `EXPOSE`. The entrypoint pins it to 9011 at every start; it moves to the next free port only if 9011 is taken then | none | as 9010 |
| 9210 | claw-sdk's agent listener, `127.0.0.1` | none | loopback |
| 9110 | the browser's CDP, `127.0.0.1` | none | loopback |
| 5900 | Xvnc, `127.0.0.1`. The host API's display and fits use it | none | loopback |
| `/run/neo/host.sock` | the host API | the socket's group, then the token | host processes in `NEO_HOST_GID` |

Nothing listens on 6080: this image serves no noVNC viewer.

### Exposure rule

- The agent ports 9010 and 9011 have no authentication. Whoever reaches them can run arbitrary Playwright
  JavaScript in the owner's logged-in browser.
- Every process in the container's network namespace reaches both, including the host's supervisor
  and any agent process. If untrusted processes share it, restrict their access, for example
  with uid firewall rules.
- With `NEO_MCP_REMOTE=true`, every peer that can route to the container reaches them too: other containers
  on the same Docker network, or hosts in a VPC.
- Publishing a port on `127.0.0.1` only hides it from other machines. It does not isolate peers inside the
  container network.
- Standalone with remote on, run the container on its own Docker network, with nothing untrusted attached.

### The host must

- mount a persistent `/data`, and run one container per `/data`;
- give Chromium shared memory: `--shm-size=1g`, or `--disable-dev-shm-usage` in `NEO_ARGS`;
- keep `NEO_MCP_REMOTE=false` unless it publishes 9010; then publish it on `127.0.0.1`, on a dedicated
  network, and never publish 9011, 9110, 9210 or 5900;
- firewall 9010 and 9011 (and 9110, 9210, 5900) for any local uid that must not drive the browser, and
  prefer an allow-list of loopback ports for such a uid, since 9011 moves if something else holds it;
- start the entrypoint as root (it renumbers neo, owns `/data`, writes Chromium policies and `/run/neo`,
  then drops to neo); `docker run --user` is not supported. It needs SETUID, SETGID, CHOWN, FOWNER,
  DAC_OVERRIDE and KILL, and works under `no-new-privileges`;
- allow at least 20 s to stop: SIGTERM quits the browser through CDP so cookies are saved;
- provide the token file and the group if it wants the host API.

`NEO_SANDBOX=false` (the default) runs Chromium with `--no-sandbox`, because Docker's default seccomp and
AppArmor profiles deny the user namespaces its sandbox needs. A host that sets `NEO_SANDBOX=true` must allow
them, as the Neo image's README describes.

### Storage

| Path | Owner, mode | Holds | Keep it? |
|---|---|---|---|
| `/data` | neo, 0700 | the folders below | yes |
| `/data/profile` | neo | the browser profile: cookies and logins (`--password-store=basic`) | yes |
| `/data/browserclaw` | neo | claw-sdk's `claw-sdk/trace.db` (runs, operations, screenshots, request-id history), `claw-sdk/outputs/` (the copy of each run's downloads and its reply's blobs, which the host reads; downloads removed with the run after `trace.retentionSeconds`, 7 days by default; blobs written under `trace.values: "off"` removed after ten minutes), `claw-sdk/settings.json` (a host's startup settings or changes through `/v1/settings`, loaded at every server start) and `logs/`. An image with an older store schema moves a newer store aside (`trace.db.broken-<ms>`) and starts empty | yes |
| `/data/downloads` | neo | downloads, where the browser saves them and `download.path()` points; Neo never deletes them | yes |
| `/run/neo` | neo, group `NEO_HOST_GID`, 2750 | `host.sock` | no; set up at every start |

A `requestId` sent with a run is answered from `trace.db` for 10 minutes after the run ends, across
restarts, so a host can retry a call whose reply it lost without running it twice.

To save a PDF shown inline, request the original URL's bytes in its signed-in page with
`page.evaluate()` and `fetch()` where same-origin or CORS permits. For a generated PDF, `page.pdf()`
returns bytes. Returning those bytes from `run` yields an inline result or a host-readable blob when
large. It does not create a download or accept a `path` option. For an actual download, await
`download.path()` before importing its host endpoint.

### Stop and restart

`tini` forwards SIGTERM to the entrypoint, which quits the browser over CDP (`neo-quit 9110`); the browser
stops claw-sdk with SIGTERM. Then the display stops and the container exits 0, in about a second. A browser
that exits is restarted by the entrypoint, and a server that exits is relaunched by the browser. A run in
flight at shutdown is cut. A relaunched server gives each task back the tabs it held while the browser
kept them (saved in `tab-holders` beside the trace); after a browser restart, no tab holds a task.

### Saved sessions

For session restore and "reopen closed tab", Chromium saves each tab's page state in
`/data/profile/Default/Sessions` (`Session_*` and `Tabs_*`). That state includes the values typed into the
tab's boxes, in plain UTF-16: a username, a one-time code, a card number and its security code, in a
cross-site frame too. Passwords and boxes marked `autocomplete=off` are not saved. The values survive
the tab's close, a site-data clear and a restart. When the browser restores the tab, it puts them back
into the boxes.

`NEO_CLEAR_SESSIONS=true` removes that folder before every start of the browser:
- the container's first start;
- each restart after the browser exits: a crash, `neo-quit`, or a kill;
- a host that kills every process in the container and runs the entrypoint again.

The folder is removed only once no browser process runs on the profile. The entrypoint waits up to
10 s for the last one to go. If a browser is still running after that, or the folder cannot be removed,
no browser starts: the container stops with the reason in its log, rather than starting a browser that
would restore the values. Its restart policy starts it again, and that start, with every process gone,
clears the folder. Nothing else in the profile is touched: cookies (a session-cookie sign-in stays signed in), saved
logins, `Local State` and `Preferences` all stay. The cost is the restored tabs: the browser opens one
New Tab, and "reopen closed tab" has nothing to reopen. It applies in hosted mode only. Unset, or
`false`, the browser keeps its sessions as before.

### Product analytics

The SDK server, `browseros-claw-sdk`, sends no product analytics. The browser sends its own usage metrics
to PostHog. In hosted mode the entrypoint starts it with `--disable-browseros-metrics`, which stops them
only in a browser built with that switch. The browser `browser.lock` pins today (0.52.1.2) does not have it
and ignores it, so its metrics still leave the container until a browser with the switch is pinned. Chromium's
own background services (component updates, GCM, Safe Browsing, network time) are a separate matter: this
switch does not touch them, and they stay on for now.

## CAPTCHA keys

The published image carries a CapSolver and a 2Captcha key, as production's Neo image does, at the
default paths above (mode 0400, Neo's uid; under other `PUID`/`PGID` the entrypoint re-owns them with
Neo's home). **They are extractable by anyone who pulls the image.** With them, `neo.solveCaptcha`
detects the page's CAPTCHA, tries its free paths first, and then has a paid provider solve it
(CapSolver, then 2Captcha), at most `BROWSEROS_CAPTCHA_PAID_TASK_LIMIT` paid tasks (100) per server
start. No file door a script has reads or overwrites a key file, even with `/` listed.

`neo.solveCaptcha` is offered to scripts once a key file is readable by Neo's uid when Neo starts;
without one, the method is unlisted and the server logs `neo.solveCaptcha is off`. Each variable says,
per provider:

- **unset**: the image's own key (none in a local or pull request build);
- **empty** (`-e BROWSEROS_CAPSOLVER_API_KEY_FILE=`): that provider is off, whatever the image carries;
  both empty turn solving off;
- **a path**: the host's own key file, in place of the image's.

When you point a provider at another server (`BROWSEROS_CAPSOLVER_API_URL`, `BROWSEROS_2CAPTCHA_API_URL`),
pass your own key file or an empty variable for it too: otherwise the image's key goes to that server.

A host provides its own key in one of two ways:

- **Write it as a file** at the default path before starting the entrypoint, as root:
  `install -D -m 0400 -o <PUID> -g <PGID> <key> /home/neo/.config/browseros/captcha/capsolver-api-key`.
- **Mount it read-only outside `/home/neo`** and point the variable at it:
  `-v ./capsolver.key:/run/neo-keys/capsolver-api-key:ro -e BROWSEROS_CAPSOLVER_API_KEY_FILE=/run/neo-keys/capsolver-api-key`.
  Not inside `/home/neo`: the entrypoint gives that whole tree to Neo's uid at every start, and a
  read-only mount there stops the container.

To bake keys into an image of your own, as the published one does, pass them to `build.sh`:

```sh
build.sh --capsolver-key-file ./capsolver.key --twocaptcha-key-file ./2captcha.key
```

They become 0400 files owned by neo in the `neo-sdk-keyed` target, `provenance.json` names them in
`captchaKeys` (provider names only), and the image gets the label `com.browseros.neo-sdk.captcha-keys=baked`.
Anyone who has that image can read them. That stage is rebuilt without its cache every time, so a rotated key never survives in a
cached layer, and a keyless build never mounts a key at all.

## Use it from a host

```sh
# in a BrowserOS checkout, packages/browseros-agent
crates/neo-sdk/neo-server/docker/build.sh --platform linux/amd64
# in the host's Dockerfile
FROM neo-sdk:0.52.1.2-<commit>-amd64
COPY settings.json /etc/neo/settings.json
```

Call `/usr/local/bin/neo-entrypoint` from the host's supervisor as root, with the `/data`
layout and neo user described above. Pass `NEO_HOST_GID` and `CLAW_HOST_TOKEN_FILE`, allow
`CLAW_*` through any `env -i` filter, add the socket's group to the host's supervisor,
and include 9011 in the firewall rules for Neo's ports.

A host can write `/data/browserclaw/claw-sdk/settings.json` before the entrypoint starts.
The file holds one `PUT /v1/settings` body with no envelope. Every setting is optional.

```json
{"tabs":{"idleCloseSeconds":300},"trace":{"values":"off","screenshots":"off","retentionSeconds":3600}}
```

Write under another name in the same folder and rename, or write while Neo is stopped.
At entrypoint startup Neo takes ownership of the state folder and saved file, setting the
file to mode 0600. This also works for a host that runs Neo under another PUID and PGID,
and for a state folder that is a symlink into another volume. The root link stays in place
and an existing folder keeps its mode. Inside the state root, the entrypoint never writes
through a symlink; a saved-file symlink is replaced by the fixed file's atomic rename.

A derived image uses `COPY settings.json /etc/neo/settings.json` as above. A host can instead
mount that fixed file read-only. At every entrypoint start, it atomically replaces the saved
file with the fixed file's bytes. A copy failure or nonregular fixed path is logged and startup
continues with the saved file. The image ships no settings file.

Neo reads the saved file before its first run, even without a usable host token.
No file means normal defaults. An unreadable or malformed file uses defaults with
`trace.values` and `trace.screenshots` off. Invalid values for either recording switch use
off; an invalid `trace` group turns both off. Invalid retention keeps its default, and unknown
names are skipped with a warning. The settings and ranges are in [the server's README](../README.md#settings).

After startup, Neo owns the file. Use `PUT` or `PATCH /v1/settings` for running changes.
API changes survive a server or browser restart. When the entrypoint starts again, a fixed
file replaces those changes. Without a fixed file, the saved changes remain.

Neo keeps one blank tab of its own open at all times (`owner: "neo"` in `GET /v1/tabs`), so closing
every other tab never ends the browser. A host does not need a keeper tab of its own.
Once the owner browses in that tab it is theirs again, and Neo opens a new blank one.

## Host API integration

The SDK host API describes runs, operations, and browser tabs. It does not serve claw-server's hosted session routes or a recording endpoint. Host requests use the authenticated host listener, separate from the agent's MCP port. The [host API contract](../../neo-sdk-api/openapi.yaml) defines each route and response.

A Go host can dial the Unix socket through `http.Transport.DialContext`. The URL host is ignored on that connection, so a URL such as `http://neo/v1/runs` works.

Send optional `requestId`, `taskId`, and `hostData` in `_meta["com.browseros.neo/run"]`. Neo returns its own run id in that key's `.id`. A request id scopes replay to its task when a task id is present, or across Neo otherwise. The guarantee lasts while Neo retains the request history. Retention removes that history and permits the id to run again.

For MCP runs with ordinary value recording, a repeated request gets the exact reply for ten minutes after the run ends, including across a server restart. Later repeats receive `duplicate_request` while the history remains. With `trace.values: "off"`, the exact reply survives in memory for ten minutes, but a repeat after a server restart receives `duplicate_request`. The [server's run contract](../README.md#the-run-contract) describes joins, client disconnects, and task selection.

Tasks group a piece of work across runs. They do not create Chromium tab groups or restrict which tabs a script may use. A run without a host or script task opens a new tab when its built-in `page` is first used. Give agents [agent.d.ts](../../agent.d.ts) for task selection and the walkthrough across runs.

Choose the host read for the question you need to answer.

| Question | Host request |
|---|---|
| Which runs belong to this task? | `GET /v1/runs?taskId={taskId}` |
| Did a request start after its reply was lost? | `GET /v1/runs?requestId={requestId}&taskId={taskId}` |
| What happened most recently? | `GET /v1/trace?taskId={taskId}&tail=20` |
| Has one run finished its operation and screenshot work? | `GET /v1/runs/{runId}?wait=60`, then inspect `complete` |
| Which runs acted on a tab? | `GET /v1/runs?tab={tabId}` |
| Which tabs are open now? | `GET /v1/tabs` |
| What does a tab show now? | `GET /v1/tabs/{tabId}/screencast` or `/video` |
| Did a tab close? | `GET /v1/events?tab={tabId}`, with `Last-Event-ID` for resumption |

Use `since` and `until` to filter the run list, and `cursor` and `limit` to page through it. There is no session-wide live or ended status, or SDK equivalent of the old session `client` and `q` filters. Each run has its own status and completeness. The newest operation with a `tabId` shows the last tab that operation targeted. Use `/v1/tabs` for a fresh inventory. A `stream.reset` event requires reading state again.

Current package names and responsibilities are in the [SDK package map](../../README.md). Custom `CLAW_LOG` directives for the host API use the `neo_http` tracing target.

## Keep a tab while a task waits

When an approval wait starts, send `PUT /v1/tabs/{tabId}/keep` with `{}`. Release it with `DELETE /v1/tabs/{tabId}/keep` when the wait ends. This replaces periodic reads that exist only to prevent idle cleanup. Add `ttlSeconds` when the keep needs a time limit.

A keep affects only idle cleanup. It is independent of settings, locks, fits, and takeovers, and agent operations remain available. Use a lock or takeover to pause agent actions. Successful keep and release requests each write an info log line. They create no run, operation, or event. The [keep contract](../../neo-sdk-api/openapi.yaml) defines replacement, expiry, restart, and missing-tab behavior.

## Read an audit timeline

One request returns a page of runs with their operations.

```http
GET /v1/runs?taskId=checkout&limit=20&fields=summary&internal=false
```

Runs appear newest first, and each run's operations appear in start order. Follow `nextCursor` for older runs. `fields=summary` also works on run detail and trace reads. `internal=false` omits Neo's own subscription operations while retaining explicit script listeners that have a recorded call site. Defaults are `fields=full&internal=true`.

A summary omits stored argument and result values, code, host data, downloads, error text, and inline screenshots. It keeps page URLs, titles, and tab identity. URLs and titles can contain values, so the host applies its own redaction before presenting an audit view. A summary changes only the read response. Full reads, stored downloads, blobs, and screenshot endpoints retain their access. Use `hasScreenshot` to find images for separate reads.

## Import downloads and large results

Wait for the download event and file completion before returning a file for the host to import.

```js
const [download] = await Promise.all([
	page.waitForEvent('download'),
	page.getByRole('button', { name: 'Download', exact: true }).click(),
]);
return { path: await download.path(), filename: download.suggestedFilename() };
```

The returned path names the file on Neo's disk, under `/data/downloads` in this image. The browser chooses its actual filename, including a suffix such as ` (1)` on a collision. Read the returned path instead of reconstructing it from the suggested filename. A later run can upload that path with `locator.setInputFiles(path)`. `download.saveAs(path)` copies the file. `download.failure()` returns `null` or a failure reason such as `canceled`.

`GET /v1/runs/{runId}` lists the downloads handed to the script. `GET /v1/runs/{runId}/downloads/{index}` serves Neo's completed copy with the host token. A file still being written when the run ends has no stored bytes. The host copy follows trace retention, while the browser's original file remains in the downloads folder.

A result larger than the 256 KiB inline limit can replace byte values with `{"$neo":"blob","id":"b0","size":N}`. Read their bytes through `GET /v1/runs/{runId}/blobs/{id}`. Text does not spill to a blob. Return large text as `Buffer.from(text)` so it can use that path. A result that still exceeds the limit fails `script / result_too_large`.

Blob lifetime follows the recording policy. Ordinary blobs follow run retention. Blobs written under `trace.values: "off"` remain fetchable for ten minutes after their write, including across a restart, and the next minute upkeep removes them.

## Type a host's secret

Use `POST /v1/tabs/{tabId}/fill/check` to inspect inputs without a value. The host decides which secret each input may receive, then sends the approved facts as `expect` with `POST /v1/tabs/{tabId}/fill`. Name each input by a snapshot ref or a ref from `locator.ref()`. `expect.origin` names the input's own frame origin. The [fill contract](../../neo-sdk-api/openapi.yaml) defines outcomes, request replay, locks, and privacy limits. Read `outcome` before deciding whether another attempt is safe.

The host typing route keeps values out of its own records and applies its tab screenshot policy. The page still holds the typed value. Page-world evaluation and page-controlled content can disclose it, so the host retains its own disclosure policy.

This route types into text inputs. Dropdown selection and navigation to a URL containing a token stay in the host's explicit flow. Browser saved form state is separate from typing and can restore a value later. See [Saved sessions](#saved-sessions) for this image's cleanup option.

## Older SDK clients

Clients of older SDK images must stop using the removed privacy calls and fields.

| Old call or field | Current contract |
|---|---|
| `POST /v1/tabs/{tabId}/private`, `POST /v1/tabs/{tabId}/sweep`, `POST /v1/secrets` | These routes are absent. Use locks or takeovers to block agent actions, and the host fill route to type a secret. |
| `tab.private`, `Event.privacy`, `TraceTail.visuals` | These are absent. Read each operation's `hasScreenshot` for a stored image. |
| Host API response codes `page_private`, `withheld`, `privacy_unavailable` | These are no longer produced. A missing or legacy withheld screenshot returns `404 not_found`. An image removed by retention returns `410 expired`. |

Clients accept response fields and extensible enum values they do not know, as defined by the [API compatibility rules](../../neo-sdk-api/openapi.yaml).

With value recording on, ordinary script arguments remain their original values, subject to storage limits. Password, card, and code targets do not cause argument redaction or withhold the new run's script text. The host fill route has its own protected record. Current `codeWithheld: "unavailable"` marks code that was never stored, such as a run interrupted by a restart.

Older trace rows remain as recorded. An argument recorded as `[redacted]` stays that way. An older run with `codeWithheld: "sensitive_input"` can still have no code. Neo does not reconstruct missing values.

### Trace recording policies

`trace.values` applies when each operation or run ends. A run can contain operations recorded under different settings. Inspect the per-record `values: "off"` marker. An absent marker means ordinary recording, and changing the setting leaves existing records unchanged.

For blobs written with values off, the ten-minute expiry survives a crash before the run's trace end commits. Startup restores the expiry, and minute upkeep removes due blobs. Downloads, page metadata, and Neo's separately configured screenshots keep their existing rules. See [Keep values out of Neo's trace](#keep-values-out-of-neos-trace) to set both value and screenshot policies.

## Run it alone, for a quick look

```sh
docker network create neo-sdk-only
docker run -d --name neo-sdk --network neo-sdk-only --shm-size=1g -e NEO_MCP_REMOTE=true \
  -p 127.0.0.1:9010:9010 -v "$PWD/neo-data:/data" "$TAG"
claude mcp add --transport http neo-sdk http://localhost:9010/mcp
```

Without `NEO_MCP_REMOTE=true` the published port is refused, because Docker's port forwarding arrives from
outside the container. That is the default on purpose.

Startup settings can come from the file above without a token. To read or change them while Neo runs, add a token and the host
API's TCP address (development only: TCP is checked by the token alone):

```sh
(umask 022 && openssl rand -hex 32 > neo-token)
docker run ... -v "$PWD/neo-token:/run/neo-keys/host-token:ro" -e CLAW_HOST_TOKEN_FILE=/run/neo-keys/host-token \
  -e CLAW_HOST_TCP=0.0.0.0:9300 -p 127.0.0.1:9300:9300 "$TAG"
curl -H "Authorization: Bearer $(cat neo-token)" http://127.0.0.1:9300/v1/settings
```

## Bump the browser

`browser.lock` pins the browser's version and each architecture's `.deb` URL and sha256. Take all five
values from the release's `release.json`:

```sh
v=0.52.1.2
curl -fsSL "https://cdn.browseros.com/releases/browserclaw/$v/linux/release.json" \
  | jq '.artifacts | {amd64: .x64_deb, arm64: .arm64_deb} | map_values({url, sha256})'
```

`browser.lock` keeps the `https://` form of each URL.

Then build and run `accept.sh` on both architectures. `accept.sh`'s `host-api` check fails if a browser
stops passing its environment on to the server.

### Keep values out of Neo's trace

Before the first run, set `{"trace":{"values":"off","screenshots":"off"}}` through
`PATCH /v1/settings`. Run and operation records keep metadata and say `values: "off"`; the
host's live reply remains complete. Neo retains download copies until trace retention.
Result blobs written under off remain fetchable for ten minutes, including across a server
restart, and the next minute upkeep removes them. Existing trace records are unchanged.
`trace.values` governs Neo's own records and log lines; other libraries can print values when
the host raises `CLAW_LOG`, so a host that needs off keeps its default `info,rmcp=warn` filter.
