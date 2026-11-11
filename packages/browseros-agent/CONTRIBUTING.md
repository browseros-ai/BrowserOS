# Contributing to BrowserOS neo

The agent platform for [BrowserOS neo](../../README.md), the browser your agents drive. Start at the [root contributing guide](../../CONTRIBUTING.md) if you have not already, then come back here.

## What you will be working on

BrowserOS neo is two pieces that run together.

| Project | What it is | Stack |
|---|---|---|
| [`apps/claw-app`](apps/claw-app/) | The cockpit. The new tab where you watch agents work, replay sessions, and manage connections | WXT + React extension, Tailwind v4, base-ui and Radix primitives, shadcn (base-vega), TanStack Query and Table, xyflow, Rive |
| [`apps/claw-server`](apps/claw-server/) | The backend. The MCP endpoint agents connect to, plus the API behind the cockpit | Rust. axum for HTTP, sea-orm with migrations, rmcp for MCP, clap, posthog-rs |

`claw-server` builds on shared crates in [`crates/`](crates/): `browseros-cdp` for Chrome DevTools Protocol bindings, `browseros-core` for primitives, `browseros-mcp` for the MCP server, `claw-api` for wire types, and `harness-integrations` for the managed AI coding harness integrations.

The wire types are generated on both sides. [`packages/claw-api`](packages/claw-api/) holds the TypeScript DTOs and [`packages/claw-api-client`](packages/claw-api-client/) the contract-typed HTTP client, both generated from the same contract as the Rust `claw-api` crate. Do not hand edit generated files; run `bun run codegen:claw-api` instead.

## Prerequisites

| Tool | Why | Install |
|---|---|---|
| **Bun** | The package manager and runtime. Version pinned in `package.json` | `curl -fsSL https://bun.sh/install \| bash` |
| **Go** | The dev supervisor is a Go program compiled on every run | `brew install go` |
| **Rust** | `claw-server` is built and run with cargo | `brew install rustup && rustup-init` |
| **BrowserOS neo** | The supervisor launches the installed app | [Download](https://cdn.browseros.com/download/BrowserOS_neo.dmg) |
| **Docker** | Only if you change the API contract. `codegen:claw-api` runs the generator in a pinned container | [Docker Desktop](https://www.docker.com/products/docker-desktop/) |

The dev loop is macOS only today. The supervisor resolves the browser through a hard-coded `/Applications/...` path, so Linux and Windows contributors can install dependencies and run the checks, but cannot launch the browser yet.

## Setup

```bash
cd packages/browseros-agent
bun install
cp .env.development.example .env.development
```

Everything in `.env.development` is optional for a first run. It holds analytics keys, Sentry DSNs, and port overrides, none of which you need to get the dev loop working. Copy it and move on.

`bun run dev:setup` does the same install with a frozen lockfile, which is what CI does.

## Run it

```bash
bun run dev:claw:watch:new
```

That starts three processes and launches the browser against them:

```
apps/claw-app            wxt                        the cockpit, with hot reload
apps/claw-app            bun run dev:web            the web preview
apps/claw-server         cargo run -p claw-server
```

A file watcher also tracks `apps/claw-server/src`, its `Cargo.toml`, and its test fixtures, and restarts the Rust server whenever they change. The first cargo build takes a while; later ones reuse compiled dependencies.

Stop everything with `bun run dev:stop`.

### `:new` versus plain

There are two variants and the difference matters more than the name suggests.

| | `bun run dev:claw:watch` | `bun run dev:claw:watch:new` |
|---|---|---|
| Ports | Preferred: CDP 9000, server 9200, extension 9300 | Random, in the 9000 to 9999 range |
| Browser profile | A persistent dev profile per checkout | A fresh temporary directory each run |
| Claw state | `~/.browserclaw-dev`, shared across worktrees | A fresh `browserclaw-dev-*` directory in the system temp directory |
| On start | **Kills anything holding those ports, and any browser already using that profile** | Only its own server port, cleared just before launch |
| Reach for it when | You want your logins and settings to survive a restart | Running more than one at once, testing first-run behaviour, or not wanting to disturb a running instance |

`:new` is the safer default while you are getting oriented, though it is not entirely hands-off: the server process clears its own port immediately before launching, so a process that grabs that port in the gap after it was picked will still be killed. Both variants lock the browser profile and the Claw state directory. Starting another watch with the same Claw state directory waits for its previous watcher to stop its children, even across worktrees. A shutdown timeout returns an error rather than reopening the same database during cleanup.

Plain watch reuses Claw settings and session history across worktrees. To select another Claw state directory, set `BROWSERCLAW_DIR`; it takes precedence for both commands:

```bash
BROWSERCLAW_DIR=~/.browserclaw-copy-prod bun run dev:claw:watch
```

The supervisor logs the selected browser profile and Claw state directory at startup. `dev:cleanup` removes temporary directories, including those created by `:new`, and preserves `~/.browserclaw-dev` and other state outside the temp directories. `--only-temps` also stops watch owners before deleting their directories; it skips explicit port cleanup. Existing state nested inside an older browser profile is left in place; point `BROWSERCLAW_DIR` at it to resume it.

## Rust build storage

Plain Cargo keeps intermediates and executables in this checkout's `target/`.
Dev incremental compilation is disabled to bound disk growth; set
`CARGO_INCREMENTAL=1` to opt in. The watch supervisor keeps its explicit private
`target/dev-watch` directory.

On macOS or Linux with Python 3, optionally install the Cargo pool shim. Put an
existing bin directory before real Cargo on `PATH` first, then run from this repo:

```bash
python3 packages/browseros-agent/tools/cargo-pool/cargo-pool.py install --bin-dir "$HOME/.local/bin"
python3 packages/browseros-agent/tools/cargo-pool/cargo-pool.py status --bin-dir "$HOME/.local/bin"
python3 packages/browseros-agent/tools/cargo-pool/cargo-pool.py uninstall --bin-dir "$HOME/.local/bin"
```

Install/uninstall are idempotent. Installation refuses an incorrectly ordered
`PATH` or an existing unrelated executable. It copies the shim, so it also covers
old worktrees and the internal mirror without editing them. Uninstall removes
only that copy; it leaves all build output in place.

The shim pools `build`/`b`, `check`/`c`, `test`/`t`, `clippy`, `rustc`, `rustdoc`,
and `doc`/`d`, plus `bench --no-run`. It leaves `run`, running benchmarks, custom subcommands/aliases, and other workspaces
alone. Each checkout prefers its last slot and waits if its other Cargo owns it.
On a handoff, Cargo cleans all previous local packages, including path dependencies,
while retaining registry/git dependencies. If the old checkout disappeared or its
package layout changed, the slot is wiped. Output modes with custom profiles,
release/cross targets or documentation are wiped on handoff or mode changes. Executables still land in
the checkout's `target/`; running them does not occupy a slot.

| Environment variable | Default | Meaning |
|---|---|---|
| `BROWSEROS_CARGO_POOL_ROOT` | `$CARGO_HOME/build/browseros-pool` (`~/.cargo` by default) | Pool directory |
| `BROWSEROS_CARGO_POOL_SLOTS` | `8` | Maximum concurrent Cargo commands |
| `BROWSEROS_CARGO_POOL_MAX_BYTES` | `3758096384` (3.5 GiB) | Per-slot eviction threshold |

A supervisor holds an OS lease until Cargo exits, including when the foreground
shim is killed. Cargo/test descendants never inherit the lease. Failed or killed Cargo commands
also terminate their private compiler process group and invalidate partial output. Slots above the
size limit are wiped before use and on exit, even for the same checkout: at rest,
the default pool holds at most **28 GiB (30.1 GB)** plus small metadata. Active
builds can temporarily exceed this by their in-flight output; one invocation's
peak output is not capped. Final executables in checkout `target/` directories,
Cargo's download cache, and the watch's private directories are outside this bound.
Reducing the slot count reclaims idle excess slots and waits for active excess
slots to finish before starting more work.

An explicit `CARGO_BUILD_BUILD_DIR`, `CARGO_TARGET_DIR`, or `--target-dir` bypasses
the pool. So do `--config` overrides and help/version requests. The shim passes
these commands to Cargo unchanged. For an opt-out that also overrides the old
shared setting in a pre-pool checkout, use:

```bash
CARGO_BUILD_BUILD_DIR="$PWD/target" cargo build --workspace
```

The CI test runner still spawns `cargo` with its existing arguments and exit status;
it needs no pool-specific behavior. CI uses normal checkout-local Cargo output,
and the Docker release caches its explicit `target/neo-docker` directory.

## Before you open a PR

```bash
bun run check        # lint, typecheck and fallow
bun test             # TypeScript suites
bun run test:rust    # the Rust workspace
bun run lint:rust    # clippy, warnings are errors
bun run fmt:rust     # rustfmt check
```

Lint and formatting are Biome. `bun run lint:fix` applies what it can.

If you changed the API contract, regenerate and commit the output:

```bash
bun run codegen:claw-api
bun run test:claw-api-contract
```

## Handy extras

| Command | What it does |
|---|---|
| `bun run dev:claw-onboard` | The onboarding flow on its own |
| `bun run dev:reset` | Wipe the dev profile and start clean |
| `bun run dev:cleanup` | Stop dev processes and free the ports |

## Where things live

```
apps/
  claw-app/              cockpit extension
    entrypoints/         WXT entrypoints
    screens/             one folder per screen
    components/          shared UI
    modules/             cross-cutting infrastructure
    lib/                 utilities
  claw-server/           Rust backend
    src/
    tests/
  claw-onboard/          first-run onboarding (Vite)
crates/                  shared Rust crates
packages/                shared TypeScript packages
tools/dev/               the Go dev supervisor
```

## Getting help

[Discord](https://discord.gg/YKwjt5vuKr) · [Slack](https://dub.sh/browserOS-slack) · [Discussions](https://github.com/browseros-ai/BrowserOS/discussions)

PR conventions, the CLA, and the browser build all live in the [root contributing guide](../../CONTRIBUTING.md).
