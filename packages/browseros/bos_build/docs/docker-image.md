# Neo Docker image

`ghcr.io/browseros-ai/neo` runs BrowserOS neo on a virtual display, streams it over
noVNC, and exposes Neo's MCP server. One tag covers `linux/amd64` and `linux/arm64`.
The release workflow installs this commit's claw-server and optional provider keys
into that same image.
The user guide is [`docker/neo/README.md`](../../docker/neo/README.md); this page is
for building and publishing it.

## Where things live

| Path | Role |
|---|---|
| `docker/neo/Dockerfile` | The image. Picks the `.deb` for each architecture from the version's public `release.json` and checks its sha256, so it builds without `bos_build` |
| `docker/neo/entrypoint.sh` | Container supervisor: Xvnc, openbox, websockify, Neo restart loop, profile setup, clean shutdown |
| `docker/neo/smoke.py` | End-to-end check from the host: viewer, MCP from outside the container, hosted tool set, an agent tab |
| `docker/neo/captcha_smoke.py` | Checks CAPTCHA tool availability with hosted mode on/off, including keyless builds |
| `bos_build/release/docker_image.py` | `build_image`: preflight that each architecture was published, tag policy, buildx invocation |
| `bos_build/cli/release_docker.py` | `browseros release docker`, a thin wrapper |
| `.github/workflows/release-docker.yml` | Build servers, verify both native images, then push one multi-architecture image |

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

The CLI keeps the package's bundled server unless `--server-dir <dir>` supplies
`<dir>/amd64/browseros-claw-server-rs` and/or `<dir>/arm64/browseros-claw-server-rs`.
Every requested platform must have a binary. The workflow supplies both from this
commit's Rust builds. Direct Docker builds use `--build-context neo-server=<dir>`.
The server build keeps the normal release's required `CLAW_POSTHOG_KEY` and
optional `CLAW_SENTRY_DSN` configuration.

## Provider keys

The workflow reuses the existing protected `neo-captcha` GitHub environment and
its `CAPSOLVER_API_KEY` and `TWOCAPTCHA_API_KEY` secrets. Both are optional. The
environment's name stays unchanged so its stored secrets and branch protections
continue to apply; dispatch releases on `main`.

Keys enter Docker through BuildKit secret mounts, then are deliberately copied
into the main image as neo-owned `0400` files in
`/home/neo/.config/browseros/captcha` (`0700`). **Anyone who can pull the image can
extract the keys. The `neo` package is public.** BuildKit keeps input values out
of command arguments; it does not hide the installed files from image users.

For local builds, pass `--capsolver-key-file <file>` and/or
`--twocaptcha-key-file <file>`. The equivalent environment variables are
`BROWSEROS_CAPSOLVER_API_KEY_FILE` and `BROWSEROS_2CAPTCHA_API_KEY_FILE`. Without
key files, the image contains no provider credentials and does not offer
`solve_captcha`. With a key, that tool appears only with hosted mode enabled.

The CLI always rebuilds the final credentials stage, including for keyless builds:
BuildKit does not include secret contents in cache keys. Browser installation
layers remain cached. For direct Docker builds, pass `--no-cache-filter neo`
and `--secret id=capsolver_api_key,src=<file>` and/or
`--secret id=twocaptcha_api_key,src=<file>`.

Rotate a key in the existing environment, then rerun the Docker release:

```sh
gh secret set CAPSOLVER_API_KEY --env neo-captcha --repo browseros-ai/BrowserOS < /secure/new-capsolver-key
gh secret set TWOCAPTCHA_API_KEY --env neo-captcha --repo browseros-ai/BrowserOS < /secure/new-2captcha-key
```

Existing images retain old keys. Consumers must pull the rebuilt image and
recreate their containers to receive the new files.

## Publish

```bash
gh workflow run release-docker.yml --ref main -f version=0.52.0.1
gh workflow run release-docker.yml --ref main -f version=0.52.0.1 -f latest=true
gh workflow run release-docker.yml --ref main -f version=0.52.0.1 -f push=false
```

The workflow builds this commit's server for both architectures, then builds and
tests each image natively: amd64 on `ubuntu-latest`, arm64 on `ubuntu-24.04-arm`.
Both run the viewer/MCP smoke check and require correct `solve_captcha` availability
with hosted mode on and off. Each mode also toggles bundled extensions on/off
using the same data volume and checks their installed state. Both verification jobs must pass before publication.
`push=false` stops after verification without publishing an image.
`verify_paid=true` additionally requires a paid Google reCAPTCHA demo PASS on amd64
and spends provider credit. Containers and temporary key files are always removed.

Publication rebuilds both architectures with the same server artifacts and key
configuration, then pushes them with `GITHUB_TOKEN`. Tag `latest` only once the version has been
promoted (`browseros release publish`); a draft should never become `latest`.

From a workstation instead, after `docker login ghcr.io` with a token that has
`write:packages`, and with a buildx builder that can build arm64 and amd64:

```bash
uv run browseros release docker --version 0.52.0.1 --push [--latest]
```

## First publish

The existing `neo` GHCR package is public and pulls without logging in, including
any embedded provider key files. The release does not change package visibility.

`browseros release docker --push` links the package to this repository by putting an
`org.opencontainers.image.source` annotation on the pushed index (it also requests a
provenance attestation so a single-platform push has an index to annotate). The
Dockerfile's label of the same name is not enough on its own: GitHub reads the
index, and the label only reaches the per-platform images. This source link
identifies the repository; it does not retroactively grant a workflow permission
to push an existing package.

## Workflow access to an existing package

Publishing requires both the job's `packages: write` token scope and the package's
permission for the publishing repository. In the
[neo package settings](https://github.com/orgs/browseros-ai/packages/container/neo/settings),
**Manage Actions access** must list `browseros-ai/BrowserOS` with **Role: Write**
(or an existing Admin grant). Add the repository if it is missing, then choose
Write. Repository source metadata and access for individual users are separate
from this Actions grant.

When the grant is missing, `docker login` and builds can succeed, but the push
fails with `denied: permission_denied: write_package` even when the runner reports
`Packages: write`. Requesting a GHCR registry token with `pull,push` scope can also
succeed without authorizing the upload. Verify the fix with an actual write using
the workflow's `GITHUB_TOKEN`, then rerun the release workflow. The workflow uses
that short-lived token; workstation publishing credentials have separate access.

GitHub documents package Actions access in
[Configuring a package's access control and visibility](https://docs.github.com/en/packages/learn-github-packages/configuring-a-packages-access-control-and-visibility#ensuring-workflow-access-to-your-package).

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
- **claw-server runs in hosted mode** (`BROWSERCLAW_HOSTED_MODE=1`): agents get the browser
  tools without skills, human help or `name_session`, and name tasks with a `taskName`
  argument. The entrypoint also turns human help off through `/api/v1/settings/agent` on
  every start, before the container reports healthy, for claw-servers that predate the mode.
- **The server updater is disabled by the entrypoint.** It cannot replace the
  packaged/overlaid server that passed the release checks, even when `NEO_ARGS` is
  overridden. Server changes ship through a rebuilt image.
  Startup clears only persisted OTA ready selectors, keeping downloaded binaries
  and application data, so older profiles also select the image's server.
