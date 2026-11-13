# The Neo SDK image

A Linux image of the BrowserOS neo browser and this commit's SDK server, `browseros-claw-sdk`. Agents get
one MCP tool, `run` (Playwright JavaScript against the browser), and the host gets the `/v1` host API on a
Unix socket. It is built for hosts that build on it, such as zuzu's cell image (`FROM neo-sdk:<tag>`), and
it runs on its own too. It is not production's image: that is the default Neo image
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
crates/neo-sdk/neo-server/docker/build.sh --platform linux/amd64   # zuzu's cells
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
An image for another architecture than the Docker engine's gets the static `identity` check only, because
booting it would be emulated. CI runs both scripts natively on amd64 and arm64
(`.github/workflows/neo-sdk-image.yml`) on every pull request and push that changes the image's inputs,
with the supervisor's and the scripts' own tests; the accept logs and `provenance.json` are its artifacts.

## Publish

Only `neo-sdk-image.yml` publishes this image, and only to `ghcr.io/browseros-ai/neo-sdk`, never to
production's `ghcr.io/browseros-ai/neo`. A pull request or a push (to `main` too) builds and tests only.
It publishes the two images `accept.sh` passed, unchanged, when asked:

```sh
gh workflow run neo-sdk-image.yml --ref <branch> -f publish=true -f confirm=neo-sdk   # :<version>
git tag neo-sdk-v0.1.0 && git push origin neo-sdk-v0.1.0                              # :<version> and :0.1.0
```

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
| `BROWSEROS_CAPSOLVER_API_KEY_FILE`, `BROWSEROS_2CAPTCHA_API_KEY_FILE` | `/home/neo/.config/browseros/captcha/capsolver-api-key`, `…/2captcha-api-key` | CAPTCHA provider keys, read if the files exist. The image ships none ([CAPTCHA keys](#captcha-keys)). |
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
- Every process in the container's network namespace reaches both. In zuzu's cell that includes mirrord,
  the harness and the agent, so only the host's uid filter keeps them apart.
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
| `/data/browserclaw` | neo | claw-sdk's `claw-sdk/trace.db` (runs, operations, screenshots, request-id history), `claw-sdk/outputs/` (the copy of each run's downloads and its reply's blobs, which the host reads; removed with the run after `trace.retentionSeconds`, 7 days by default), `claw-sdk/settings.json` (what the host set through `/v1/settings`, loaded at every start) and `logs/`. An image with an older store schema moves a newer store aside (`trace.db.broken-<ms>`) and starts empty | yes |
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
flight at shutdown is cut.

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

The image is keyless. `neo.solveCaptcha` is offered to scripts once a key file is readable by Neo's uid
when Neo starts; without one, the method is unlisted. A host provides a key in one of two ways:

- **Write it as a file** at the default path before starting the entrypoint, as root:
  `install -D -m 0400 -o <PUID> -g <PGID> <key> /home/neo/.config/browseros/captcha/capsolver-api-key`.
- **Mount it read-only outside `/home/neo`** and point the variable at it:
  `-v ./capsolver.key:/run/neo-keys/capsolver-api-key:ro -e BROWSEROS_CAPSOLVER_API_KEY_FILE=/run/neo-keys/capsolver-api-key`.
  Not inside `/home/neo`: the entrypoint gives that whole tree to Neo's uid at every start, and a
  read-only mount there stops the container.

To bake keys into an image instead, as the Neo image does, pass them to `build.sh`:

```sh
build.sh --capsolver-key-file ./capsolver.key --twocaptcha-key-file ./2captcha.key
```

They become 0400 files owned by neo in the `neo-sdk-keyed` target, and the image gets the label
`com.browseros.neo-sdk.captcha-keys=baked`. Anyone who has that image can read them, so never push it to a
public registry. That stage is rebuilt without its cache every time, so a rotated key never survives in a
cached layer, and a keyless build never mounts a key at all.

## Use it from zuzu

```sh
# in a BrowserOS checkout, packages/browseros-agent
crates/neo-sdk/neo-server/docker/build.sh --platform linux/amd64
# in zuzu's cell/Dockerfile
FROM neo-sdk:0.52.1.2-<commit>-amd64
```

zuzu keeps calling `/usr/local/bin/neo-entrypoint` from its own supervisor, as root, with the same `/data`
layout and the neo user. It passes `NEO_HOST_GID` and `CLAW_HOST_TOKEN_FILE` (its `env -i` filter must let
`CLAW_*` through), adds the socket's group to mirrord, and adds 9011 to its firewall's Neo ports.

How Neo behaves is set on the host API, not in a file or the environment: once Neo answers, and before
its first run, mirrord sends `PUT /v1/settings` with everything it wants, such as
`{"tabs":{"idleCloseSeconds":300}}`. Neo saves it under `/data` and loads it again at every start: when
the browser restarts the server, and in a new container on the same `/data`. mirrord still sends its `PUT`
at every start of its own, so the saved set is what it wants now, not what an older mirrord or a hand
`PATCH` left there. The settings and their ranges are in [the server's README](../README.md#settings).

## Run it alone, for a quick look

```sh
docker network create neo-sdk-only
docker run -d --name neo-sdk --network neo-sdk-only --shm-size=1g -e NEO_MCP_REMOTE=true \
  -p 127.0.0.1:9010:9010 -v "$PWD/neo-data:/data" "$TAG"
claude mcp add --transport http neo-sdk http://localhost:9010/mcp
```

Without `NEO_MCP_REMOTE=true` the published port is refused, because Docker's port forwarding arrives from
outside the container. That is the default on purpose.

Settings need the host API, which needs a token. To read or change them by hand, add a token and the host
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
