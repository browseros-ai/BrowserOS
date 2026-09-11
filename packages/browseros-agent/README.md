# BrowserOS Agent

The agent platform behind both browsers. [BrowserOS neo](../../README.md) is `claw-app` plus `claw-server-rust`, and [BrowserOS](../../README.BrowserOS.md) is `app` plus `server`.

## Start here instead

Setup, the dev loop, what each project is built with, and where the code lives are all in the contributing guides. This file does not repeat them.

- **[Contributing to BrowserOS neo](CONTRIBUTING.md)**
- **[Contributing to BrowserOS](CONTRIBUTING.BrowserOS.md)**
- **[Root contributing guide](../../CONTRIBUTING.md)** for the browser build, the CLA and PR conventions

What follows is reference material for people already set up: how environment configuration works, how servers are configured at startup, and how production artifacts are built.

## Environment configuration

Two root env files, both gitignored:

| File | Used by |
|---|---|
| `.env.development` | Local development, tests, app and server runs, codegen inputs |
| `.env.production` | Release builds and upload scripts |

Their tracked templates, `.env.development.example` and `.env.production.example`, are **generated** from [`@browseros/shared/env/registry`](packages/shared/src/env/registry.ts). Run `bun run env:examples` after changing the registry. CI drift-checks the generated output, so a registry change without a regenerated example fails the build.

Existing checkouts that still have old per-app env files can run `bun run env:migrate` to merge those values into the root files.

### Sections

| Section | File | Purpose |
|---|---|---|
| `dev-tools` | `.env.development` | Codegen and local tooling inputs such as `CDP_PROTOCOL_JSON` and `BROWSEROS_BINARY` |
| `app` | `.env.development` | Extension and local BrowserOS launch settings: dev ports, public Vite values, source-map upload, optional GraphQL schema path |
| `claw` | `.env.development` | BrowserOS neo overrides: API URL, user-data dir, CDP port, `BROWSERCLAW_DIR` |
| `server` | `.env.development`, `.env.production` | Server config URL, telemetry, Sentry, `NODE_ENV`, log level, local server test settings |
| `upload` | `.env.production` | Cloudflare R2 credentials and bucket for artifact uploads |

Production build and upload scripts read `.env.production` plus exported process env through `@browseros/shared/env/*`, and **exported process env wins**. A missing required value fails with an error naming the key, the section, and the file.

## Server sidecar config

Both servers read their startup configuration from a sidecar JSON file passed as `--config <path>`, rather than from flags or env. `tools/dev`, dogfood, and Chromium-managed launches all generate this file.

| Field | Description |
|---|---|
| `ports.server` | HTTP server port: MCP endpoints, chat, health |
| `ports.cdp` | Chromium CDP port, which the server connects to as a client |
| `ports.proxy` | Browser proxy port emitted by Chromium |
| `directories.resources` | Packaged resources root |
| `directories.execution` | Runtime execution, log and config directory |
| `instance.*` | Optional browser and client metadata |

Default local ports are `9100` for the server, `9000` for CDP, and `9300` for the extension. The dev supervisor may pick others; see the contributing guides.

## Typecheck: a gotcha worth knowing

`bun run typecheck` uses the native TypeScript 7 compiler, the Go-native build. Unlike an editor's bundled classic TypeScript, it does **not** implicitly include every `@types/*` package it finds. Each tsconfig has to list its ambient type packages in `compilerOptions.types`; the root defaults to `["node", "bun"]`.

A new package that omits this looks green in your editor and then fails CI with `Cannot find name 'Bun'` or `Cannot find name 'process'`. Add the names to that package's `types` array. For editor parity, install your editor's native TypeScript 7 support.

## Production artifacts

```bash
bun run build                 # server and agent
bun run build:server          # server resource artifacts for every target, uploads zips to R2
bun run build:claw-server     # five-platform BrowserOS neo Rust resources, uploads zips to R2
bun run build:agent           # the extension
```

`build:server` emits under `dist/prod/server/<target>/`, with zips at `dist/prod/server/`. The underlying script takes options directly:

```bash
bun scripts/build/server.ts --target=all
bun scripts/build/server.ts --target=darwin-arm64,linux-x64
bun scripts/build/server.ts --target=all --manifest=scripts/build/config/server-prod-resources.json
bun scripts/build/server.ts --target=all --no-upload
```

### BrowserOS neo Rust artifacts

The Rust builder runs on macOS and produces the same five resource zips as `release-claw-server.yml`: macOS ARM64 and x64, Linux ARM64 and x64, and Windows x64. Darwin targets use Cargo and Xcode, Linux targets use Zig with a glibc 2.17 floor, and Windows uses the MSVC ABI through `cargo-xwin`.

One-time host toolchain:

```bash
xcrun --find clang || xcode-select --install
brew install rustup zig llvm cmake nasm
rustup-init
export PATH="$(brew --prefix llvm)/bin:$PATH"

cargo install --locked cargo-zigbuild --version 0.23.0
cargo install --locked cargo-xwin --version 0.23.0
rustup target add \
  aarch64-apple-darwin \
  x86_64-apple-darwin \
  aarch64-unknown-linux-gnu \
  x86_64-unknown-linux-gnu \
  x86_64-pc-windows-msvc
```

The build preflights every selected target before compiling and repeats the install command for anything missing. `cargo-xwin` downloads and caches the Microsoft CRT and Windows SDK on its first run.

**Uploads are the default.** Without `--ci` or `--no-upload`, a build pushes its zips to R2. Pass one of them for a local build.

```bash
bun scripts/build/claw-server-rust.ts --target=all --ci        # all five, no R2 credentials needed
bun scripts/build/claw-server-rust.ts --target=darwin-arm64 --no-upload    # one target, local only
```

## Focused test groups

The full suites are `bun test` and `bun run test:main`. When you want a narrower loop:

```bash
cd apps/server
bun run test:tools         # the MCP tool surface
bun run test:cdp           # CDP and browser control
bun run test:integration   # end to end
```

## License

AGPL-3.0, same as the rest of the repo. See [LICENSE](../../LICENSE).
