# Private Neo CAPTCHA image

`ghcr.io/browseros-ai/neo-captcha` extends the public Neo image with this
commit's claw-server and optional CapSolver/2Captcha key files. It is a
**private** package. Anyone able to pull it can extract the keys, including
from image layers; grant package access accordingly. Never make it public.

The server updater is disabled so it cannot replace the overlaid server.
Keys enter builds only through BuildKit secrets, become neo-owned `0400`
files in `/home/neo/.config/browseros/captcha` (`0700`), and never appear in
build arguments, environment values, labels, provenance, SBOMs or exported
build caches. No secrets means no key files. Builds disable layer reuse;
when building manually, use `--no-cache` after changing secret mounts.

## Release

Run on `main`: the `neo-captcha` GitHub environment allows only protected
branches. Its secrets are `CAPSOLVER_API_KEY` and `TWOCAPTCHA_API_KEY`.

```sh
gh workflow run release-docker-captcha.yml --repo browseros-ai/BrowserOS \
  --ref main -f neo_version=0.51.2.1 -f verify_paid=false -f push=true
```

Tags are `<neo_version>-<short_sha>` and `latest`, for amd64 and arm64.
`verify_paid=true` also requires a Google demo PASS and spends provider credit.
`push=false` builds and verifies without publishing keyed layers; on first
use it still creates the **keyless** `bootstrap` tag.

Every release checks anonymous registry access. Public packages and uncertain
responses block keyed publication. If bootstrap becomes public, a maintainer
must make **neo-captcha** private in its GitHub package settings, then rerun.
Do not change the public `neo` package's visibility.

Before publishing either architecture, the workflow checks both native
images: amd64 on `ubuntu-latest`, arm64 on `ubuntu-24.04-arm`. Each requires
healthy containers and `solve_captcha` present with hosted mode on, absent
with hosted mode off. The paid demo runs only on amd64. Both verification
jobs must pass before publication. **Until the hosted-gate PR (W9)
merges, the off check intentionally fails and nothing keyed is pushed.**
Verification prints health JSON, tool names and the paid runner table; never
container logs. Server build jobs receive no environment secrets.

Rotate either key, then rebuild (existing images retain their old keys):

```sh
gh secret set CAPSOLVER_API_KEY --env neo-captcha --repo browseros-ai/BrowserOS < /secure/new-capsolver-key
gh secret set TWOCAPTCHA_API_KEY --env neo-captcha --repo browseros-ai/BrowserOS < /secure/new-2captcha-key
```

## Run

Authenticate Docker to GHCR with a token that has `read:packages`, then:

```sh
docker pull ghcr.io/browseros-ai/neo-captcha:latest
docker run --rm --shm-size=1g -p 127.0.0.1:6080:6080 -p 127.0.0.1:9010:9010 \
  -e BROWSERCLAW_HOSTED_MODE=1 ghcr.io/browseros-ai/neo-captcha:latest
# Desktop mode: same command with -e BROWSERCLAW_HOSTED_MODE=0 (no solve_captcha).
```

The viewer is at `http://localhost:6080`, MCP at `http://localhost:9010/mcp`.
Use a fresh data volume for each verification. See [Neo](../neo/README.md)
for persistence and other runtime options. Keep the updater flag when adding
`NEO_ARGS`. To test the release guards locally:

```sh
uv run --no-project --with PyYAML==6.0.3 python -m unittest discover -s packages/browseros/docker/neo-captcha -p 'test_*.py'
```
