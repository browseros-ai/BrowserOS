# @browseros/build-tools

Builds agent image tarballs, publishes release artifacts to R2, and hydrates the local dev cache for agent tarballs.

The BrowserOS VM is defined by a committed Lima template at `template/browseros-vm.yaml`. There is no custom disk build step; `limactl` consumes the template directly at runtime.

## Setup

```bash
cp packages/build-tools/.env.sample packages/build-tools/.env
bun install
```

## Dev loop against the Lima template

Requires `limactl` on PATH. It is bundled with the server; for bare-worktree use, install Lima with Homebrew.

```bash
brew install lima
```

```bash
limactl start \
  --name browseros-vm-dev \
  packages/browseros-agent/packages/build-tools/template/browseros-vm.yaml

limactl shell browseros-vm-dev nerdctl info

SOCK="$(limactl list browseros-vm-dev --format '{{.Dir}}')/sock/containerd.sock"
test -S "$SOCK"

bun run --filter @browseros/build-tools build:tarball -- --agent openclaw --arch arm64
limactl shell browseros-vm-dev nerdctl load -i "$(ls dist/images/openclaw-*-arm64.tar.gz | head -1)"

limactl delete --force browseros-vm-dev
```

## Upload bundled Lima runtime files

BrowserOS ships the Lima files needed by production server artifacts. Upload them from the upstream Lima release tarballs:

```bash
cd packages/browseros
uv run browseros upload lima --version v2.1.1 --dry-run
uv run browseros upload lima --version v2.1.1
```

The upload stores four R2 objects:

```text
artifacts/vendor/third_party/lima/limactl-darwin-arm64
artifacts/vendor/third_party/lima/lima-guestagent.Linux-aarch64.gz
artifacts/vendor/third_party/lima/limactl-darwin-x64
artifacts/vendor/third_party/lima/lima-guestagent.Linux-x86_64.gz
```

Server resource staging uses relative manifest keys such as `third_party/lima/limactl-darwin-arm64`; set `R2_DOWNLOAD_PREFIX=artifacts/vendor` in `apps/server/.env.production` so those keys resolve to the uploaded objects.

The final server resource zip must contain real files, not a nested Lima runtime archive. Lima finds its runtime data by walking from `bin/limactl` to the sibling `share/lima` directory:

```text
resources/bin/third_party/lima/bin/limactl
resources/bin/third_party/lima/share/lima/lima-guestagent.Linux-aarch64.gz
resources/bin/third_party/lima/share/lima/lima-guestagent.Linux-x86_64.gz
```

`lima-additional-guestagents` is not required for BrowserOS native macOS artifacts. The core Darwin release tarballs already contain the native Linux guest agents used by our VM.

Build a server resource artifact and smoke test the bundled prefix:

```bash
cd ../browseros-agent
bun run build:server:test

TMP_PREFIX="$(mktemp -d /tmp/browseros-lima-prefix.XXXXXX)"
TMP_HOME="$(mktemp -d /tmp/browseros-lima-home.XXXXXX)"
RESOURCES_DIR="dist/prod/server/darwin-arm64/resources"

mkdir -p "$TMP_PREFIX/bin" "$TMP_PREFIX/share/lima"
cp "$RESOURCES_DIR/bin/third_party/lima/bin/limactl" "$TMP_PREFIX/bin/limactl"
cp "$RESOURCES_DIR/bin/third_party/lima/share/lima/lima-guestagent.Linux-aarch64.gz" "$TMP_PREFIX/share/lima/"

LIMA_HOME="$TMP_HOME" "$TMP_PREFIX/bin/limactl" create --tty=false --name=browseros-smoke \
  packages/build-tools/template/browseros-vm.yaml
LIMA_HOME="$TMP_HOME" "$TMP_PREFIX/bin/limactl" delete --force browseros-smoke

rm -rf "$TMP_PREFIX" "$TMP_HOME"
```

## Build an agent tarball

The BrowserOS VM uses containerd + nerdctl. This host-side tarball builder still requires `podman` to pull and save OCI archives for release packaging.

```bash
bun run --filter @browseros/build-tools build:tarball -- --agent openclaw --arch arm64
```

## Smoke test an agent tarball

```bash
bun run --filter @browseros/build-tools smoke:tarball -- --agent openclaw --arch arm64 --tarball ./dist/images/openclaw-2026.4.12-arm64.tar.gz
```

## Emit a manifest

```bash
bun run --filter @browseros/build-tools emit-manifest -- --dist-dir packages/build-tools/dist
```

Publish workflows can update one agent slice at a time. Sliced publishing requires an existing R2 `vm/manifest.json` baseline; bootstrap first releases with `--slice full`.

```bash
bun run --filter @browseros/build-tools emit-manifest -- --slice agents:openclaw --merge-from https://cdn.browseros.com/vm/manifest.json
```

## Sync the dev cache

```bash
NODE_ENV=development bun run --filter @browseros/build-tools cache:sync
```

Pulls the published manifest and tarballs from R2 (`cdn.browseros.com/vm/`). Development cache files land under `~/.browseros-dev/cache/vm/images/`. Production-mode cache files land under `~/.browseros/cache/vm/images/`.

## Seed the dev cache from a local build

```bash
NODE_ENV=development bun run --filter @browseros/build-tools dev:seed:tarball
```

`dev:seed:tarball` hardcodes `arm64` (all devs are on Apple Silicon), builds the configured agent tarball, skips R2 entirely, and writes an arm64-only manifest + tarball into `~/.browseros-dev/cache/vm/`. It refuses to run unless `NODE_ENV=development`. Use this when you want to test the server against the latest configured agent tarball without publishing.
