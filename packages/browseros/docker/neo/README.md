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
| MCP tool set (see below) | `-e BROWSERCLAW_MODE=desktop` | `BROWSERCLAW_MODE` | `hosted` |
| Chromium sandbox (see [Security](#security)) | `-e NEO_SANDBOX=true` | `NEO_SANDBOX` | `false` |
| Extra Neo command-line flags | `-e NEO_ARGS="--lang=de"` | `NEO_ARGS` | empty |

Ports inside the container are always 6080 and 9010; change only the host side.

`BROWSERCLAW_MODE=hosted` gives agents the browser tools only. Skills (`save_skill`,
`mark_skill_run`) and human help (`request_human_help`, `await_human_help`) are hidden, since
your agent's own harness keeps its workflows and asks you for help itself. Instead of calling
`name_session`, an agent names its task with an optional `taskName` argument on any tool call.
`desktop` restores the full tool set of the BrowserOS app.
The viewer's screen follows the browser tab's size once it connects, so `NEO_SCREEN`
only matters until then.

## Read what your agent did

In `hosted` mode, port 9010 also serves `/v1`, a read-only API for your backend. Its
OpenAPI spec is
[`contracts/hosted-api/openapi.yaml`](../../../browseros-agent/contracts/hosted-api/openapi.yaml).
Within `/v1`, fields are only ever added, never renamed or removed.

| Request | Returns |
|---|---|
| `GET /v1/sessions?client=&since=&until=&q=&cursor=&limit=` | Sessions, most recently active first |
| `GET /v1/sessions/{id}?include=screenshots&wait=60` | One session and every tool call, with each screenshot inlined |
| `GET /v1/sessions/{id}/recording` | The page recording as rrweb events, one entry per page |

- **The session id** is the `Mcp-Session-Id` header your MCP client got from `initialize`, or
  `_meta["com.browseros.neo/session"]` on a tool result.
- **Fetch once the session is `complete`.** Screenshots and the recording land a few seconds
  after the calls. A session ends when your client closes it (`DELETE /mcp`) or after 30
  idle minutes. `wait=60` holds the request until it is complete.
- **Records are kept for 7 days.**

## Your data

Everything Neo keeps lives under `/data`:

```
/data/profile      the Neo profile: logins, cookies, history, settings
/data/browserclaw  agent data: sessions, recordings, skills, agent settings
/data/downloads    Neo's Downloads folder
```

- **Sign in once.** Logins survive `docker compose down` / `up` and image upgrades.
- **One container per data folder.** Two containers on one profile corrupt it.
- **On Linux, match the owner.** Set `PUID`/`PGID` to your user (`id -u`, `id -g`) so files
  in the folder stay yours. Docker Desktop on a Mac handles this for you.
- **A desktop profile can't be copied in with its logins.** Desktop Neo on a Mac encrypts
  cookies with the macOS Keychain, which Linux can't read.

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
outside the container, the human-help setting, and an agent opening a tab.

```sh
python3 smoke.py                          # default ports
python3 smoke.py --vnc-port 7080 --mcp-port 7010
```

## Build it yourself

The Dockerfile reads the version's published `release.json` from the BrowserOS CDN, picks the
package for the architecture being built, and checks its sha256:

```sh
docker build --build-arg NEO_VERSION=0.51.1.4 -t neo .
```

To test a Neo build that isn't on the CDN (a CI artifact, say), put its `.deb` alone in a
folder and pass that folder as the `neo-deb` build context:

```sh
docker build --build-context neo-deb=/path/to/folder -t neo .
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
