# Neo Docker image

`ghcr.io/browseros-ai/neo` runs BrowserOS neo on a virtual display, streams it over
noVNC, and exposes Neo's MCP server. One tag covers `linux/amd64` and `linux/arm64`.
The user guide is [`docker/neo/README.md`](../../docker/neo/README.md); this page is
for building and publishing it.

## Where things live

| Path | Role |
|---|---|
| `docker/neo/Dockerfile` | The image. Picks the `.deb` for each architecture from the version's public `release.json` and checks its sha256, so it builds without `bos_build` |
| `docker/neo/entrypoint.sh` | Container supervisor: Xvnc, openbox, websockify, Neo restart loop, profile setup, clean shutdown |
| `docker/neo/smoke.py` | End-to-end check from the host: viewer, MCP from outside the container, hosted tool set, an agent tab |
| `bos_build/release/docker_image.py` | `build_image`: preflight that each architecture was published, tag policy, buildx invocation |
| `bos_build/cli/release_docker.py` | `browseros release docker`, a thin wrapper |
| `.github/workflows/release-docker.yml` | Dispatch workflow: build amd64, smoke test, push both architectures |

## Prerequisite: a Linux release with both architectures

The image installs released packages. A version can be published only when its
`releases/browserclaw/<version>/linux/release.json` lists both `x64_deb` and
`arm64_deb`, i.e. the Neo release ran with `linux_arch=all` and uploaded to R2.
`browseros release docker --push` checks this before building and names what is missing:

```
❌ Neo 0.51.1.4 has no x64_deb on the CDN
```

## Build locally

From `packages/browseros`:

```bash
# This machine's architecture, loaded into local Docker (arm64 on Apple silicon).
uv run browseros release docker --version 0.52.0.1

# A package that isn't on the CDN, e.g. a CI artifact. Loaded locally only.
uv run browseros release docker --deb ~/Downloads/BrowserOS_neo_v0.52.0.1_arm64.deb

# Then run and check it.
cd docker/neo
NEO_VERSION=0.52.0.1 docker compose up -d --wait
python3 smoke.py
docker compose down
```

A local `.deb` named like a release package (`BrowserOS_neo_v<version>_<arch>.deb`)
supplies the version and platform itself; otherwise pass `--version`.

## Publish

```bash
gh workflow run release-docker.yml -f version=0.52.0.1              # version tag only
gh workflow run release-docker.yml -f version=0.52.0.1 -f latest=true
```

The workflow builds the amd64 image, runs `smoke.py` against it, then pushes both
architectures with `GITHUB_TOKEN`. Tag `latest` only once the version has been
promoted (`browseros release publish`); a draft should never become `latest`.

From a workstation instead, after `docker login ghcr.io` with a token that has
`write:packages`, and with a buildx builder that can build arm64 and amd64:

```bash
uv run browseros release docker --version 0.52.0.1 --push [--latest]
```

## First publish

A new ghcr package starts private. After the first push, open the package's settings
on GitHub and change its visibility to Public (the org must allow public packages).
Public images pull without logging in.

`browseros release docker --push` links the package to this repository by putting an
`org.opencontainers.image.source` annotation on the pushed index (it also requests a
provenance attestation so a single-platform push has an index to annotate). The
Dockerfile's label of the same name is not enough on its own: GitHub reads the
index, and the label only reaches the per-platform images. The link is what lets the
workflow's `GITHUB_TOKEN` push; a package created without it must be connected in its
package settings first.

## Runtime notes for maintainers

- **Ports inside the container are fixed:** noVNC 6080, MCP proxy 9010
  (`--browseros-proxy-port`), CDP 9110 and claw-server 9210 (pinned, loopback only).
- **All state lives under `/data`.** The profile is `/data/profile`. claw-server's database,
  recordings, skills and agent settings are `/data/browserclaw` (`BROWSERCLAW_DIR`); its
  default, `~/.browserclaw`, is not on the volume.
- **Remote MCP needs a profile setting.** Docker's port forwarding doesn't arrive from
  loopback, so the entrypoint merges `browseros.server.allow_remote_in_mcp: true` into
  the profile's `Local State` before Neo starts.
- **Shutdown quits Neo over CDP.** `neo-quit` sends `Browser.close`; Chromium's SIGTERM
  path skips flushing. Neo on Linux still records a crash even after a clean quit,
  so Neo starts with `--hide-crash-restore-bubble`.
- **The sandbox is off by default** (`NEO_SANDBOX=false`): Docker's default seccomp
  profile blocks the user namespaces it needs. With the sandbox off, a managed policy
  hides Chromium's `--no-sandbox` warning bar.
- **claw-server runs in hosted mode** (`BROWSERCLAW_MODE=hosted`): agents get the browser
  tools without skills, human help or `name_session`, and name tasks with a `taskName`
  argument. The entrypoint also turns human help off through `/api/v1/settings/agent` on
  every start, before the container reports healthy, for claw-servers that predate the mode.
