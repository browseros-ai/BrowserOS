# Docker CAPTCHA E2E

From `packages/browseros-agent`:

```sh
bun run test:captcha-docker-e2e -- --keys none --expect-tool-missing
bun run test:captcha-docker-e2e -- --keys placeholder --keep
bun run test:captcha-docker-e2e -- --keys real --paid --out ../../.llm/captcha-paid
```

Requires Bun, Docker Desktop with Linux arm64 support, and the merged
`test:captcha-e2e` runner. No Chromium build. The command builds the published
Neo image, compiles the checkout's claw-server, overlays it, checks health and
the running executable's SHA-256, lists MCP tools, then runs every fixture/demo.
It checks the executable again afterward. Results and step timings are saved
under `../../.llm/captcha-docker-results` by default.

`--neo-version VERSION` overrides the pinned release. `--rebuild` rebuilds the
base image. `--keep` preserves the container and its temporary mounts, including
on failure; open its printed noVNC URL to watch. It prints a cleanup command.
Without `--keep`, the container and temporary files are removed on exit.
`--clean` also removes the base and overlay images; it conflicts with `--keep`.
Other flags, such as `--only no-captcha` or `--solve-timeout 150000`, go to W3.

## Server build and keys

On macOS, automatic builder selection uses installed `cargo-zigbuild` and Zig:

```sh
CARGO_TARGET_DIR=/Volumes/extssd/cargo-target/captcha-docker-e2e \
  cargo install --locked cargo-zigbuild --version 0.23.0
rustup target add aarch64-unknown-linux-gnu
```

The build uses `cargo zigbuild --release --locked --target
aarch64-unknown-linux-gnu.2.17 -p claw-server --bin browseros-claw-server-rs`.
`--builder container` forces the `rust:1.96-bookworm` fallback; it mounts the
checkout read-only and keeps target/registry caches on the external disk.
Override `CARGO_TARGET_DIR` when needed. Remove that directory after testing
when the compilation cache is no longer useful.

`none` mounts no keys. `placeholder` mounts two fake key files. `real` mounts
each owner's file read-only, without reading/copying its contents. Sources are
`BROWSEROS_CAPSOLVER_API_KEY_FILE` and `BROWSEROS_CAPMONSTER_API_KEY_FILE`, or
`~/.config/browseros/captcha/{capsolver,capmonster}-api-key`. The container gets
those contract variables pointing at `/run/captcha/`. Keys never enter images
or build arguments. No paid solve is verified: the supplied CapSolver key is
rejected (`ERROR_KEY_DENIED_ACCESS`), and CapMonster has zero balance.

## Image and OTA

The default `0.51.1.4` was selected on 2026-10-22 from the Linux
[release metadata](https://cdn.browseros.com/releases/browserclaw/0.51.1.4/linux/release.json):
it has `arm64_deb`; `0.51.1.2` does not. The public GitHub releases and macOS
appcast still named `0.51.1.2`. Candidate `0.51.1.5`, `0.51.1.6` and `0.52.0*`
Linux metadata returned 404.

The package installs its server at
`/usr/lib/browserclaw/BrowserClawServer/default/resources/bin/browseros-claw-server`
(found using `find` in the image). The overlay copies the newly built Rust
binary there. Neo can choose downloaded server versions under its profile's
`.browseros/BrowserClawServer/versions/`. A fresh `/data` and
`NEO_ARGS=--disable-browseros-server-updater` prevent that during tests. Hashes
come from `/proc/PID/exe` as user `neo`, before and after testing: container root
cannot read another UID's executable under Docker's default capabilities.

The first arm64 build took about 110 seconds; the server cross-build took
178 seconds. The overlay reports 2.26 GB in `docker image ls` (28.2 MB new binary),
with base layers shared. Warm timings and the final page proof are recorded in
the PR and `proof.json`; the early run reached all ten pages with
`tool_not_found` / PASS. Docker image usage increased about 3.8 GB during setup;
temporary profiles and redundant development image tags are cleaned afterward.
