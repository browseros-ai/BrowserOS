# BrowserOS neo in Docker

Run BrowserOS neo in a container and use it from your machine:

- **See and drive Neo** in a browser tab (noVNC). Click, type, copy and paste.
- **Connect any agent** to Neo's MCP server and watch it work live in that tab.
- **Keep your profile** (logins, history, downloads) in a folder you choose.

One image runs natively on Apple silicon Macs, arm64 Linux and x86-64 Linux.

## Quick start

You need Docker (Docker Desktop on a Mac).

```sh
docker run -d --name neo --shm-size=1g --hostname neo \
  -p 127.0.0.1:6080:6080 -p 127.0.0.1:9010:9010 \
  -v "$HOME/neo-data:/data" \
  ghcr.io/browseros-ai/neo:latest
```

Or, with [`compose.yaml`](compose.yaml) from this folder:

```sh
docker compose up -d --wait
```

`--wait` returns once Neo is ready. Then:

- **Open the viewer:** <http://localhost:6080>. Neo fills the tab and resizes with it.
- **Connect an agent:** `http://localhost:9010/mcp` is a streamable-HTTP MCP server. For Claude Code:

  ```sh
  claude mcp add --transport http neo http://localhost:9010/mcp
  ```

  Ask the agent to open a page and it shows up in the viewer.

`docker logs neo` (or `docker compose logs`) prints these URLs once Neo is ready.

## Settings

With compose, put settings in a `.env` file next to `compose.yaml` or set them in the shell:

```sh
NEO_VNC_PORT=7080 NEO_MCP_PORT=7010 NEO_DATA=/mnt/neo docker compose up -d --wait
```

| Setting | `docker run` | compose / env var | Default |
|---|---|---|---|
| Viewer port on your machine | `-p 127.0.0.1:<port>:6080` | `NEO_VNC_PORT` | 6080 |
| MCP port on your machine | `-p 127.0.0.1:<port>:9010` | `NEO_MCP_PORT` | 9010 |
| Data folder | `-v <folder>:/data` | `NEO_DATA` | `./neo-data` |
| Image version | image tag | `NEO_VERSION` | `latest` |
| Owner of files in the data folder | `-e PUID=… -e PGID=…` | `PUID`, `PGID` | 1000 / 1000 |
| Starting screen size | `-e NEO_SCREEN=1920x1080` | `NEO_SCREEN` | `1600x1000` |
| Hosted mode (see below) | `-e BROWSERCLAW_HOSTED_MODE=0` | `BROWSERCLAW_HOSTED_MODE` | `1` |
| Server behaviour (see [Behaviour file](#behaviour-file)) | `-v ./neo.toml:/etc/neo/neo.toml:ro` | the `neo.toml` volume in `compose.yaml` | the image's `/etc/neo/neo.toml` |
| Chromium sandbox (see [Security](#security)) | `-e NEO_SANDBOX=true` | `NEO_SANDBOX` | `false` |
| Software graphics (see [Graphics](#graphics)) | `-e NEO_SOFTWARE_GRAPHICS=false` | `NEO_SOFTWARE_GRAPHICS` | `true` |
| Bundled BrowserOS extensions off (see [Bundled extensions](#bundled-extensions)) | `-e NEO_DISABLE_BROWSEROS_EXTENSIONS=false` | `NEO_DISABLE_BROWSEROS_EXTENSIONS` | `true` |
| Extra Neo command-line flags (see [Browser frame](#browser-frame)) | `-e NEO_ARGS="--lang=de"` | `NEO_ARGS` | empty |

Ports inside the container are always 6080 and 9010; change only the host side.

Image tags: `latest` and browser versions such as `0.52.0.1` move when a newer build is published. To stay
on one source commit, use its `<version>-<commit>` tag (e.g. `0.52.0.1-169e41f`); a rebuild of that commit
replaces it. To pin exact image contents, use the digest.
The server updater is disabled so the server stays at the version verified with
the image. Pull a newer image and recreate the container to update it.
The server's product analytics are off (`CLAW_ANALYTICS_ENABLED=0`).

`BROWSERCLAW_HOSTED_MODE=1` gives agents the browser tools only. Skills (`save_skill`,
`mark_skill_run`) and human help (`request_human_help`, `await_human_help`) are hidden, since
your agent's own harness keeps its workflows and asks you for help itself. Instead of calling
`name_session`, an agent names its task with an optional `taskName` argument on any tool call.
Neo also leaves tab layout to your agent: a page joins a tab group only when the agent passes
`groupId`, and Neo never creates, titles, collapses or closes a group itself. The
[behaviour file](#behaviour-file) can turn per-task groups back on. From Neo 0.51.2.5, the
browser in hosted mode names itself Google Chrome to websites that read its brand (User-Agent
Client Hints).
`BROWSERCLAW_HOSTED_MODE=0` restores the desktop tools and hides `solve_captcha`.
Only exact `0` and `1` are accepted. After changing the flag, run
`docker compose up -d --wait` to recreate the container with its new environment.
With `docker run`, recreate it with the new flag and the existing data volume.
The viewer's screen follows the browser tab's size once it connects, so `NEO_SCREEN`
only matters until then.

Images built with a CapSolver or 2Captcha key also offer `solve_captcha` in hosted
mode. Keyless builds omit that tool. Provider keys are embedded files: anyone who
can pull such an image can extract them. See the [build and release guide](../../bos_build/docs/docker-image.md)
for key configuration and rotation.

### Behaviour file

Server behaviour that a deployment may want to change lives in one TOML file instead of
more environment variables. The image ships `/etc/neo/neo.toml` with every key commented
out, so it documents the defaults; replace it to change them. Mount your own over it:

```sh
docker run ... -v ./neo.toml:/etc/neo/neo.toml:ro ghcr.io/browseros-ai/neo
```

or uncomment the `neo.toml` line in `compose.yaml`, or `COPY` one to `/etc/neo/neo.toml` in
an image built `FROM` Neo. A key you leave out keeps the default for the mode
(`BROWSERCLAW_HOSTED_MODE`). A key or value the server does not know keeps its default and
is logged as a warning in `docker logs`, naming the key it probably meant when it looks like a
typo, so a file written for a newer server still starts an older one. A file that is not valid
TOML stops the server with the reason in `docker logs`, and so does a
`BROWSERCLAW_CONFIG_FILE` that names a missing file. claw-server 0.0.84 and earlier stop on any
key they do not know.
Recreate the container after editing the file.

| Key | Values | Hosted default | Desktop default |
|---|---|---|---|
| `[tabs] grouping` | `"explicit"`: a page joins a group only when the agent passes `groupId`. `"auto"`: Neo keeps each task's pages in a `<client>/<task>` group it titles, collapses and closes. | `"explicit"` | `"auto"` |

The server logs the values it runs with at startup (`server behaviour`).

### Browser frame

The viewer is for watching agents and for taking over a login, code or payment, so Neo shows
a minimal browser frame: horizontal tabs, blank new tabs, and a toolbar with only the back,
forward, reload and menu buttons around the address bar. The entrypoint starts Neo with
`--disable-browseros-features` and every name below. The browser applies them for that run
only, never saves them in the profile, and the viewer cannot turn them back on. This needs
Neo 0.51.2.5 or later; older browsers ignore the switch and show the full frame.

| Name | Turns off |
|---|---|
| `VerticalTabs` | Vertical tabs; the tab strip stays horizontal |
| `HomeButton` | The home button |
| `SplitTabButton` | The split-view button |
| `TabSearchButton` | The tab search button |
| `BookmarkEditing` | Adding or editing bookmarks, and the star |
| `WebAppInstall` | Installing sites as apps |
| `AvatarButton` | The profile button |
| `ExtensionsToolbar` | The extensions button |
| `NewTabPage` | The new tab page; new tabs open `about:blank` |

To keep some of these, pass your own list in `NEO_ARGS`. It replaces the image's list, because
Neo keeps the last value of a repeated switch; an empty value turns none off:

```sh
docker run ... -e NEO_ARGS="--disable-browseros-features=VerticalTabs,HomeButton" ...
```

The bookmarks bar and the window buttons have no names. Every container start turns the bar
off with a managed policy and gives the window an empty GTK button layout.

### Zoom

Neo never zooms: the page in the viewer is the page your agent works in. To see it bigger,
zoom your own browser or device.

- Neo starts with `--disable-pinch`, which stops touch pinch, and `--browseros-disable-zoom`,
  which keeps every tab at 100% whatever zoom keys, Ctrl + mouse wheel, the menu, extensions or
  saved zoom levels ask for. Browsers without that switch ignore it, Neo 0.51.2.7 among them.
- The window manager also takes the zoom keys (Ctrl + `=`, `-` or `0`, on the keypad too) and
  Ctrl + mouse wheel, which is how noVNC sends a two-finger pinch, so browsers without the switch
  never see them. Pages don't get them either, so maps and drawing tools zoom with their own
  buttons. It never sees an agent's DevTools input, and in browsers without the switch the
  menu's zoom buttons still work.
- Each time Neo starts, including after someone closed it, it forgets zoom levels the profile
  saved, per site or as a changed default.
- `--disable-pinch` also drops pinch gestures an agent sends over DevTools
  (`Input.synthesizePinchGesture`).
- `setViewport`'s `zoom` scales its mobile preview through DevTools emulation, not browser zoom,
  and still works, as does F11 fullscreen.

### Bundled extensions

Neo runs without the bundled BrowserOS extensions, since agents drive it over MCP. MCP browser
tools and user-installed extensions stay enabled. Turn the bundled ones back on for the
BrowserOS extension UI and rrweb page recordings (`/v1/sessions/{id}/recording`, which returns
no pages without them) by passing `-e NEO_DISABLE_BROWSEROS_EXTENSIONS=false` to `docker run`,
or with Compose:

```sh
NEO_DISABLE_BROWSEROS_EXTENSIONS=false docker compose up -d --wait
```

`NEO_DISABLE_BROWSEROS_EXTENSIONS=true` adds `--disable-browseros-extensions` without replacing
`NEO_ARGS`, so derived images keep their own flags. It accepts `true`/`1`/`yes`/`on` and
`false`/`0`/`no`/`off`, ignoring case; other values stop startup.
Recreate the container after changing it; `docker restart` keeps the old setting.
With an existing data volume, Neo may uninstall its bundled extensions and
install them again when re-enabled.

### Graphics

Containers and microVMs rarely have a GPU, so Neo draws with Mesa's llvmpipe, which runs on the
CPU. One setting decides what pages can use:

- `NEO_SOFTWARE_GRAPHICS=true` (the default) adds `--ignore-gpu-blocklist
  --disable-gpu-compositing`. Pages get WebGL1 and WebGL2, drawn by the CPU, so 3D-heavy pages
  cost much more CPU than on a desktop with a GPU. Compositing, page painting and video stay on
  Chromium's own software path, as in earlier images. In hosted mode, WebGL reports a desktop
  GPU rather than llvmpipe.
- `NEO_SOFTWARE_GRAPHICS=false` adds neither, so Chromium keeps its defaults: no WebGL on
  llvmpipe. Use it on a host with a supported GPU, or when CPU matters more than WebGL.

Like the other true/false settings, it accepts `true`/`1`/`yes`/`on` and `false`/`0`/`no`/`off`.
Flags in `NEO_ARGS` add to these and cannot remove them, so use the setting to opt out. Installed
desktop browsers are unchanged.

WebGPU stays unavailable: Chromium offers pages no WebGPU adapter without a GPU. To test WebGPU
anyway, add `--enable-unsafe-webgpu` to `NEO_ARGS`. Pages then get Chromium's SwiftShader
fallback, but the flag relaxes Chromium's WebGPU safety rules for every site, BrowserOS reports
the real software GPU (llvmpipe and SwiftShader) instead of the hosted GPU identity, and with
`NEO_SANDBOX=true` the viewer shows Chromium's unsupported-flag warning. Don't use it for normal
browsing.

The smoke check renders a triangle with both WebGL versions and reads it back. In hosted mode it
also checks that WebGL reports the hosted GPU identity rather than a software renderer.

From Neo 0.51.2.6, hosted mode also suppresses Runtime console and asynchronous
exception notifications to prevent inspector error formatting from invoking page
callbacks. Native MCP console reads therefore omit those messages. Execution
contexts, evaluation, synchronous evaluation errors, and bindings remain available;
`BROWSERCLAW_HOSTED_MODE=0` restores normal inspector notifications.

## Read what your agent did

In `hosted` mode, port 9010 also serves `/v1`, a read-only API for your backend. Its
OpenAPI spec is
[`contracts/hosted-api/openapi.yaml`](../../../browseros-agent/contracts/hosted-api/openapi.yaml).
Within `/v1`, fields are only ever added, never renamed or removed.

| Request | Returns |
|---|---|
| `GET /v1/sessions?client=&since=&until=&q=&cursor=&limit=` | Sessions, most recently active first |
| `GET /v1/sessions/{id}?include=screenshots&wait=60` | One session and every tool call, with each screenshot inlined |
| `GET /v1/sessions/{id}/recording` | The page recording as rrweb events, one entry per page; needs the [bundled extensions](#bundled-extensions) |
| `GET /v1/sessions/{id}/screencast?quality=low\|high&tab=` | A live MJPEG view of the tab the agent is working in; works as an `<img src>` |
| `GET /v1/sessions/{id}/video?quality=low\|high&tab=` | The same tab as live AV1 video (fragmented MP4, ~2 s behind), for Media Source Extensions |

- **The live views** stream only while you watch, and only the session's own tabs. Without
  `tab`, `/screencast` follows the agent from tab to tab. Both close when the session ends.
- **The session id** is the `Mcp-Session-Id` header your MCP client got from `initialize`, or
  `_meta["com.browseros.neo/session"]` on a tool result.
- **Fetch once the session is `complete`.** Screenshots and the recording land a few seconds
  after the calls. A session ends when your client closes it (`DELETE /mcp`) or after 30
  idle minutes. `wait=60` holds the request until it is complete.
- **Records are kept for 7 days.**

## Your data

Everything Neo keeps lives under `/data` by default:

```
/data/profile      the Neo profile: logins, cookies, history, settings
/data/browserclaw  agent data: sessions, recordings, skills, agent settings
/data/downloads    Neo's Downloads folder
```

`NEO_DATA_DIR` changes the directory **inside the container**. For example, a
derived image can set `NEO_DATA_DIR=/var/lib/mirror/browseros`; Neo then uses
`profile`, `browserclaw`, and `downloads` below that directory. Mount that directory
or a persistent parent explicitly:

```sh
docker run --rm --shm-size=1g \
  -p 127.0.0.1:6080:6080 -p 127.0.0.1:9010:9010 \
  -e NEO_DATA_DIR=/var/lib/mirror/browseros \
  -v "$HOME/neo-data:/var/lib/mirror/browseros" \
  ghcr.io/browseros-ai/neo:latest
```

The path must be absolute and must not resolve to `/`. Neo makes the data root
private (`0700`) and gives it and its three data directories to `PUID`/`PGID`;
parent directories keep their ownership and permissions. The image declares no
Docker volumes, so persistence requires an explicit mount. Compose and the
default `docker run` example already provide one at `/data`; neither changes.
`NEO_DATA` in Compose chooses the host directory. Compose keeps its existing
`/data` target; using a custom container directory requires your own Compose override.

- **Sign in once.** Logins survive `docker compose down` / `up` and image upgrades.
- **One container per data folder.** Two containers on one profile corrupt it.
- **On Linux, match the owner.** Set `PUID`/`PGID` to your user (`id -u`, `id -g`) so files
  in the folder stay yours. Docker Desktop on a Mac handles this for you.
- **A desktop profile can't be copied in with its logins.** Desktop Neo on a Mac encrypts
  cookies with the macOS Keychain, which Linux can't read.
- **Guard the folder like the logins in it.** A container has no keyring, so Neo encrypts saved
  passwords and cookies with Chromium's built-in key (`--password-store=basic`). Anyone who can
  read the folder can read them.
- **Cards stay out of the profile.** A managed policy turns payment autofill off, so Neo never
  offers to save a card. Agents fill cards from your own secure storage through the
  `payment_cards` tool, which fills a supplied card without saving it.

## Security

Neither port asks for a password. Whoever reaches them controls a browser that is
signed in to your accounts.

- **Port 6080** gives full control of Neo's screen, mouse and keyboard.
- **Port 9010** exposes Neo's whole local API, not only `/mcp`: sessions, recordings and settings.
- **Keep the `127.0.0.1:` in `-p`.** Without it, Docker publishes the port on every network
  interface, and on Linux that bypasses firewalls such as `ufw`.
- **To share it,** put your own authenticating reverse proxy in front of both ports.

**Chromium's sandbox is off by default.** It needs Linux user namespaces, which Docker's
default security profile blocks, so the image starts Neo with `--no-sandbox`. Pages then run
without Chromium's process sandbox; the container is the only wall. To keep the sandbox, opt in
and relax Docker's seccomp profile for this container:

```sh
docker run … -e NEO_SANDBOX=true --security-opt seccomp=unconfined …
```

Docker Desktop on a Mac needs only that. Ubuntu 24.04 hosts also restrict user namespaces with
AppArmor, so they may also need `--security-opt apparmor=unconfined`. In compose, add both under
`security_opt:`.

## Check that it works

[`smoke.py`](smoke.py) checks a running container from your machine: the viewer, MCP from
outside the container, the human-help setting, an agent opening a tab, the blank new tab page,
and WebGL1/2 rendering with the hosted GPU identity (see [Graphics](#graphics)).

```sh
python3 smoke.py                          # default ports
python3 smoke.py --vnc-port 7080 --mcp-port 7010
```

[`check_zoom.py`](check_zoom.py) runs inside the container. It sends a pinch and the zoom keys
the way the viewer does and fails if the page zooms (see [Zoom](#zoom)):

```sh
docker exec -i neo python3 - < check_zoom.py
```

## Build it yourself

The Dockerfile reads the version's published `release.json` from the BrowserOS CDN, picks the
package for the architecture being built, and checks its sha256. Use 0.51.2.5 or later (see
[Browser frame](#browser-frame)):

```sh
docker build --no-cache-filter neo --build-arg NEO_VERSION=0.51.2.5 -t neo .
```

To test a Neo build that isn't on the CDN (a CI artifact, say), put its `.deb` alone in a
folder and pass that folder as the `neo-deb` build context:

```sh
docker build --no-cache-filter neo --build-context neo-deb=/path/to/folder -t neo .
```

Maintainers build and publish the image with `browseros release docker`; see
[`bos_build/docs/docker-image.md`](../../bos_build/docs/docker-image.md).

## Troubleshooting

| Symptom | Fix |
|---|---|
| `port is already allocated` / `address already in use` | Another program uses 6080 or 9010. Pick other host ports with `NEO_VNC_PORT` / `NEO_MCP_PORT` (or `-p`). |
| Tabs crash with "Aw, Snap!" | Run with `--shm-size=1g` (compose sets `shm_size: 1g`). |
| Container stays `unhealthy` | `docker logs neo`. `No usable sandbox!` means `NEO_SANDBOX=true` without the security options above. |
| `permission denied` writing `/data` on Linux | Set `PUID`/`PGID` to the owner of your data folder. |
| Viewer shows "Failed to connect" | The container stopped or restarted; the page reconnects by itself once it is back. |
| `No such image` for a version | Images exist for Neo versions released for both amd64 and arm64 Linux. |
