# Contributing to BrowserOS

The agent platform for [BrowserOS](../../README.BrowserOS.md), the browser you drive with an AI agent built in. Start at the [root contributing guide](../../CONTRIBUTING.md) if you have not already, then come back here.

Working on BrowserOS neo instead? See [CONTRIBUTING.md](CONTRIBUTING.md).

## What you will be working on

BrowserOS is two pieces that run together.

| Project | What it is | Stack |
|---|---|---|
| [`apps/app`](apps/app/) | The extension. New tab, side panel chat, onboarding and settings | WXT + React extension, GraphQL with codegen and graphqlsp, AI SDK React bindings, Radix and shadcn, PostHog |
| [`apps/server`](apps/server/) | The server. The MCP tool surface plus the agent loop that runs your tasks | Bun. Hono with zod-validator, MCP served over Hono, Drizzle ORM with drizzle-kit, AI SDK across Anthropic, OpenAI, Google, Bedrock, Azure, OpenRouter and more |

The extension talks GraphQL to the server. The schema lives at [`apps/app/schema/schema.graphql`](apps/app/schema/schema.graphql) and the typed documents are generated, so run `bun run codegen:agent` after changing it rather than editing generated files.

## Prerequisites

| Tool | Why | Install |
|---|---|---|
| **Bun** | The package manager and runtime. Version pinned in `package.json` | `curl -fsSL https://bun.sh/install \| bash` |
| **Go** | The dev supervisor is a Go program compiled on every run | `brew install go` |
| **Lima** | The dev supervisor requires it and refuses to start without it | `brew install lima` |
| **BrowserOS** | The supervisor launches the installed app | [Download](https://files.browseros.com/download/BrowserOS.dmg) |

Rust is not needed for this path. It is only required for the BrowserOS neo backend.

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
bun run dev:watch
```

That starts three processes and launches the browser against them:

```
apps/app       wxt                      the extension, with hot reload
apps/app       bun run dev:web          the web preview
apps/server    bun --watch src/index.ts the server, restarting on change
```

Stop everything with `bun run dev:stop`.

### `:new` versus plain

There are two variants and the difference matters more than the name suggests.

| | `bun run dev:watch` | `bun run dev:watch:new` |
|---|---|---|
| Ports | Fixed: CDP 9000, server 9100, extension 9300 | Random, in the 9000 to 9999 range |
| Profile | A persistent dev profile that keeps its state | A fresh temporary directory each run |
| On start | **Kills anything holding those ports, and any browser already using that profile** | Nothing to kill |
| Reach for it when | You want your logins and settings to survive a restart | Running more than one at once, testing first-run behaviour, or not wanting to disturb a running instance |

`:new` is the safer default while you are getting oriented. Both take a lock on the profile, so two watch runs can never supervise the same one.

### Running the BrowserOS neo stack instead

`dev:watch` and `dev:claw:watch` are exclusive, not additive. One command runs one product. To work on neo, see [CONTRIBUTING.md](CONTRIBUTING.md).

## Before you open a PR

```bash
bun run check    # lint, typecheck and fallow
bun test         # TypeScript suites
```

Lint and formatting are Biome. `bun run lint:fix` applies what it can.

If you changed the GraphQL schema, regenerate and commit the output:

```bash
bun run codegen:agent
```

## Handy extras

| Command | What it does |
|---|---|
| `bun run start:server` | The server on its own, without the browser |
| `bun run start:agent` | The extension dev server on its own |
| `bun run dev:app-onboard` | The onboarding flow on its own |
| `bun run devtools` | AI SDK devtools against the server |
| `bun run dev:reset` | Wipe the dev profile and start clean |
| `bun run dev:cleanup` | Stop dev processes and free the ports |
| `bun run dev:manual` | Build the extension statically instead of hot reloading, for when you need to load it by hand |

## Where things live

```
apps/
  app/                   the extension
    entrypoints/         WXT entrypoints
    screens/             one folder per screen
    components/          shared UI
    modules/             cross-cutting infrastructure
    hooks/               global hooks
    lib/                 utilities
    schema/              GraphQL schema
  server/
    src/
      agent/             the agent loop
      api/               HTTP routes
      tools/             the MCP tool surface
      lib/               shared internals
    tests/
  app-onboard/           first-run onboarding (Vite)
packages/                shared TypeScript packages
tools/dev/               the Go dev supervisor
```

## Getting help

[Discord](https://discord.gg/YKwjt5vuKr) · [Slack](https://dub.sh/browserOS-slack) · [Discussions](https://github.com/browseros-ai/BrowserOS/discussions)

PR conventions, the CLA, and the browser build all live in the [root contributing guide](../../CONTRIBUTING.md).
