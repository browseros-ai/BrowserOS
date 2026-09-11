# Contributing to BrowserOS neo

The agent platform for [BrowserOS neo](../../README.md), the browser your agents drive. Start at the [root contributing guide](../../CONTRIBUTING.md) if you have not already, then come back here.

Working on the other browser instead? See [CONTRIBUTING.BrowserOS.md](CONTRIBUTING.BrowserOS.md).

## What you will be working on

BrowserOS neo is two pieces that run together.

| Project | What it is | Stack |
|---|---|---|
| [`apps/claw-app`](apps/claw-app/) | The cockpit. The new tab where you watch agents work, replay sessions, and manage connections | WXT + React extension, Tailwind v4, base-ui and Radix primitives, shadcn (base-vega), TanStack Query and Table, xyflow, Rive |
| [`apps/claw-server-rust`](apps/claw-server-rust/) | The backend. The MCP endpoint agents connect to, plus the API behind the cockpit | Rust. axum for HTTP, sea-orm with migrations, rmcp for MCP, clap, posthog-rs |

`claw-server-rust` builds on shared crates in [`crates/`](crates/): `browseros-cdp` for Chrome DevTools Protocol bindings, `browseros-core` for primitives, `browseros-mcp` for the MCP server, `claw-api` for wire types, and `harness-integrations` for the managed AI coding harness integrations.

The wire types are generated on both sides. [`packages/claw-api`](packages/claw-api/) holds the TypeScript DTOs and [`packages/claw-api-client`](packages/claw-api-client/) the contract-typed HTTP client, both generated from the same contract as the Rust `claw-api` crate. Do not hand edit generated files; run `bun run codegen:claw-api` instead.

## Prerequisites

| Tool | Why | Install |
|---|---|---|
| **Bun** | The package manager and runtime. Version pinned in `package.json` | `curl -fsSL https://bun.sh/install \| bash` |
| **Go** | The dev supervisor is a Go program compiled on every run | `brew install go` |
| **Lima** | The dev supervisor requires it and refuses to start without it | `brew install lima` |
| **Rust** | `claw-server-rust` is built and run with cargo | `brew install rustup && rustup-init` |
| **BrowserOS neo** | The supervisor launches the installed app | [Download](https://cdn.browseros.com/download/BrowserOS_neo.dmg) |

## Setup

```bash
cd packages/browseros-agent
bun install
cp .env.development.example .env.development
```

Everything in `.env.development` is optional for a first run. It holds analytics keys, Sentry DSNs, and port overrides, none of which you need to get the dev loop working. Copy it and move on.

`bun run dev:setup` does the same install with a frozen lockfile and then runs codegen, which is what CI does.

## Run it

```bash
bun run dev:claw:watch:new
```

That starts three processes and launches the browser against them:

```
apps/claw-app            wxt                        the cockpit, with hot reload
apps/claw-app            bun run dev:web            the web preview
apps/claw-server-rust    cargo run -p claw-server-rust
```

A file watcher also tracks `apps/claw-server-rust/src`, its `Cargo.toml`, and its test fixtures, and restarts the Rust server whenever they change. The first cargo build takes a while; later ones are incremental.

Stop everything with `bun run dev:stop`.

### `:new` versus plain

There are two variants and the difference matters more than the name suggests.

| | `bun run dev:claw:watch` | `bun run dev:claw:watch:new` |
|---|---|---|
| Ports | Fixed: CDP 9000, server 9100, extension 9300 | Random, in the 9000 to 9999 range |
| Profile | A persistent dev profile that keeps its state | A fresh temporary directory each run |
| On start | **Kills anything holding those ports, and any browser already using that profile** | Nothing to kill |
| Reach for it when | You want your logins and settings to survive a restart | Running more than one at once, testing first-run behaviour, or not wanting to disturb a running instance |

`:new` is the safer default while you are getting oriented. Both take a lock on the profile, so two watch runs can never supervise the same one.

### Running the BrowserOS stack instead

`dev:watch` and `dev:claw:watch` are exclusive, not additive. One command runs one product. To work on the other browser, see [CONTRIBUTING.BrowserOS.md](CONTRIBUTING.BrowserOS.md).

## Before you open a PR

```bash
bun run check        # lint, typecheck and fallow
bun test             # TypeScript suites
bun run test:rust    # the Rust workspace
bun run lint:rust    # clippy, warnings are errors
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
| `bun run devtools` | AI SDK devtools against the server |
| `bun run dev:manual` | Build the extension statically instead of hot reloading, for when you need to load it by hand |

## Where things live

```
apps/
  claw-app/              cockpit extension
    entrypoints/         WXT entrypoints
    screens/             one folder per screen
    components/          shared UI
    modules/             cross-cutting infrastructure
    lib/                 utilities
  claw-server-rust/      Rust backend
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
