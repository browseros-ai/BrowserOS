# Docker CAPTCHA E2E

From `packages/browseros-agent`:

```sh
bun run test:captcha-docker-e2e -- --keys none --expect-tool-missing
bun run test:captcha-docker-e2e -- --keys placeholder --keep
bun run test:captcha-docker-e2e -- --keys real --paid --out ../../.llm/captcha-paid
```

Requires Bun 1.4.2, Docker Desktop with Linux arm64 support, and the merged
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
Other flags, such as `--only no-captcha` or `--solve-timeout 150000`, go to the
`contracts/captcha/e2e` runner.

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

`none` mounts no keys. `placeholder` mounts one fake CapSolver file. `real`
mounts `BROWSEROS_CAPSOLVER_API_KEY_FILE` read-only, defaulting to
`~/.config/browseros/captcha/capsolver-api-key`. It also mounts
`BROWSEROS_2CAPTCHA_API_KEY_FILE` only when that file exists (default
`~/.config/browseros/captcha/2captcha-api-key`). These v1.1 contract variables
point at the corresponding files under `/run/captcha/`. Contents are never
read/copied by the harness, baked into images, or passed in build arguments.
The solver tries CapSolver, then 2Captcha, then returns `needs_human`. Paid
tests are reserved for the mediator's central run. Worker proofs use `none`
or `placeholder`; the 2Captcha path is unverified: no key yet.

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

The first base build took about 110 seconds; the cold server cross-build took
178 seconds. The container builder took 510 seconds including image/dependency
downloads. Both produced matching running-binary hashes and responsive MCP.
The overlay reports 2.26 GB in `docker image ls`, sharing base layers, with a
28.2 MB server binary.

Early pre-merge smoke measurements (final rerun follows the runner's merge):

| Step | Time |
| --- | ---: |
| Cached base lookup | 0.06 s |
| Warm server build | 1.04 s |
| Overlay image | 0.42 s |
| Container healthy | 5.35 s |
| Running binary proof | 0.08 s |
| Ten page checks, external runner | about 30.5 s |

Every early page returned `tool_not_found` / PASS. Each run records fresh
measurements in `proof.json`. Docker image usage increased about 5.9 GB while
testing both builders. Cargo/registry caches used 1.4 GB on the external disk.
Temporary profiles and redundant development image tags are cleaned afterward;
the base and useful overlay images stay cached unless `--clean` is requested.
