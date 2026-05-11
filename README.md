# Shimmy-Browser

<div align="center">
  <img src="docs/images/shimmy-wordmark.png" width="320" height="80" alt="Shimmy-Browser logo" />
</div>

**Shimmy-Browser** ([GitHub](https://github.com/Karthikprasadm/Shimmy-Browser)) is a development fork of **BrowserOS** — an open-source Chromium-based browser that runs AI agents locally and connects them to the web through the **Chrome DevTools Protocol (CDP)**. This repository contains:

1. **`packages/browseros`** — the **BrowserOS browser**: Chromium patches, Python build orchestration, packaging, and signing.
2. **`packages/browseros-agent`** — the **agent platform**: Bun server (MCP + agent loop), WXT/React extension UI, shared packages, eval harness, and CLI tooling.
3. **`packages/browseros-agent/vendor/sup-agent`** — the **Sup-agent** Git submodule: upstream skill definitions and related assets consumed by this fork (see [Sup-agent submodule](#sup-agent-submodule)).

This README is written for contributors and advanced users who want a **single map of the whole repo**: how pieces talk to each other, how to run the stack locally, how the browser is built, and how Sup-agent fits in.

---

## Table of contents

- [What you are running in practice](#what-you-are-running-in-practice)
- [High-level architecture](#high-level-architecture)
- [Repository layout](#repository-layout)
- [Prerequisites](#prerequisites)
- [Quick start (full agent stack)](#quick-start-full-agent-stack)
- [Agent platform (`packages/browseros-agent`)](#agent-platform-packagesbrowseros-agent)
  - [Apps and packages](#apps-and-packages)
  - [Ports and environment variables](#ports-and-environment-variables)
  - [Development workflows](#development-workflows)
  - [Useful scripts](#useful-scripts)
- [Browser (`packages/browseros`)](#browser-packagesbrowseros)
- [Sup-agent submodule](#sup-agent-submodule)
- [Documentation in this repo](#documentation-in-this-repo)
- [Quality checks (Biome, Lefthook)](#quality-checks-biome-lefthook)
- [Git: fork, upstream, and submodules](#git-fork-upstream-and-submodules)
- [Troubleshooting](#troubleshooting)
- [License and credits](#license-and-credits)

---

## What you are running in practice

For **local development**, you typically run three cooperating pieces:

| Piece | Role |
|--------|------|
| **Chromium (or Chrome)** | Launched by the agent dev tooling with **remote debugging** enabled so the **server** can attach over **CDP**. |
| **Agent extension (WXT + Vite)** | The BrowserOS UI: app pages, side panel, background service worker, new-tab override, etc. |
| **BrowserOS server (`apps/server`)** | A **Bun** process that exposes HTTP routes (chat, MCP, health, …), runs the **AI agent loop**, and uses CDP to drive the browser. |

The **built BrowserOS binary** (from `packages/browseros`) is what end-users install; in dev you often use a **stock Chromium** or a **downloaded BrowserOS build**, configured via `BROWSEROS_BINARY` and matching ports in `.env.development` files.

---

## High-level architecture

```
┌─────────────────────────────────────────────────────────────────┐
│  Extension UI (WXT/React) — chat, settings, Connect Apps, …      │
│  Talks to HTTP API (same host/port as local server in dev)       │
└───────────────────────────────┬─────────────────────────────────┘
                                │ HTTP (REST, SSE, MCP transports)
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│  BrowserOS Server (Bun, `apps/server`)                           │
│  • Agent loop (LLM via AI SDK)                                  │
│  • MCP tools (browser automation, filesystem, memory, …)          │
│  • Optional Klavis / external MCP integrations                   │
└───────────────────────────────┬─────────────────────────────────┘
                                │ CDP client
                                ▼
┌─────────────────────────────────────────────────────────────────┐
│  Chromium CDP endpoint (debug port)                              │
│  Exposed by the browser process (9912-style dev port in .env)   │
└─────────────────────────────────────────────────────────────────┘
```

**Important:** The server **connects to** the browser’s CDP port; the browser does not call the server. If CDP is down or the port mismatch, you will see health checks failing or “connection failed” style errors in the UI.

---

## Repository layout

```
Shimmy-Browser/
├── package.json                 # Root convenience: `bun run dev` → full stack script
├── packages/
│   ├── browseros/               # Chromium fork — patches, Python CLI, packaging
│   │   ├── build/               # `browseros` Python CLI and modules
│   │   ├── chromium_patches/    # Patches applied to Chromium source
│   │   ├── chromium_files/      # New files merged into the Chromium tree
│   │   ├── series_patches/      # Ordered patch series
│   │   ├── resources/           # Icons, entitlements, signing assets
│   │   ├── CHROMIUM_VERSION     # Pinned Chromium version
│   │   └── BASE_COMMIT          # Base Chromium commit
│   │
│   └── browseros-agent/         # Agent monorepo (Bun workspaces)
│       ├── apps/
│       │   ├── agent/           # Extension: WXT, React, entrypoints
│       │   ├── server/          # Bun server: MCP + agent loop + CDP
│       │   ├── cli/             # Go CLI (+ npm wrapper under cli/npm)
│       │   └── eval/            # Evaluation / benchmarking
│       ├── packages/
│       │   ├── agent-sdk/      # @browseros-ai/agent-sdk
│       │   ├── cdp-protocol/   # CDP typings / codegen
│       │   └── shared/         # Shared constants (limits, URLs, …)
│       ├── vendor/
│       │   └── sup-agent/      # Git submodule (see below)
│       ├── scripts/
│       │   └── dev/
│       │       └── stack.ts    # One-command dev: agent + wait CDP + server
│       ├── process-compose.yaml # Optional multi-process dev (install + server + …)
│       └── package.json         # Workspace scripts (build, test, lint, …)
├── docs/                        # Mintlify-style docs (includes submodule workflow)
├── CONTRIBUTING.md              # Upstream-oriented contributor guide
├── LICENSE                      # AGPL-3.0 (see file for exact terms)
└── README.md                    # This file
```

---

## Prerequisites

| Requirement | Notes |
|-------------|--------|
| **[Bun](https://bun.sh)** | The `packages/browseros-agent` workspace pins a **packageManager** version (see `packages/browseros-agent/package.json`). Install a matching Bun release. |
| **Git** | For submodules: `git submodule update --init --recursive`. |
| **Python 3.12+** | Only needed for **`packages/browseros`** (Chromium build CLI). |
| **Disk / RAM (browser build)** | Chromium development: on the order of **~100 GB** disk and **16 GB+ RAM** recommended upstream. |

---

## Quick start (full agent stack)

From the **repository root**:

```bash
cd packages/browseros-agent
bun install
cp apps/server/.env.example apps/server/.env.development
cp apps/agent/.env.example apps/agent/.env.development
# Edit BOTH files so BROWSEROS_SERVER_PORT, BROWSEROS_CDP_PORT (and Vite URL/port) agree.
```

Then either:

**Option A — recommended for this fork (single terminal)**

```bash
# From repo root
bun run dev
```

This runs `packages/browseros-agent/scripts/dev/stack.ts`, which:

1. Loads `packages/browseros-agent/apps/agent/.env.development`.
2. Starts **`bun run --filter @browseros/agent dev`** (WXT dev server + Chromium with the extension).
3. Waits until **CDP** responds on `http://127.0.0.1:<BROWSEROS_CDP_PORT>/json/version`.
4. Starts **`bun run --filter @browseros/server start`**.

Press **Ctrl+C** to stop both processes.

**Option B — from `packages/browseros-agent` only**

```bash
cd packages/browseros-agent
bun run dev
```

(`dev`, `start`, and `dev:stack` in that `package.json` point at the same stack script.)

**VS Code:** Run the task **“Dev: full stack (agent + browser + server)”** (`.vscode/tasks.json`) which executes `bun run dev` at the workspace root.

---

## Agent platform (`packages/browseros-agent`)

The agent monorepo follows upstream BrowserOS structure. Official package READMEs with extra detail:

- [`packages/browseros-agent/README.md`](packages/browseros-agent/README.md)
- [`packages/browseros-agent/apps/server/README.md`](packages/browseros-agent/apps/server/README.md)
- [`packages/browseros-agent/apps/agent/README.md`](packages/browseros-agent/apps/agent/README.md)

### Apps and packages

| Path | Technology | Purpose |
|------|------------|---------|
| `apps/server` | Bun, Hono, Vercel AI SDK | HTTP API, MCP surfaces, agent loop, CDP client, SQLite sessions, skills/memory tooling |
| `apps/agent` | WXT, Vite, React | Extension UI: `app` full page, side panel, background worker, new tab, onboarding, options |
| `apps/cli` | Go + npm shim | `browseros-cli` — terminal control of a running BrowserOS instance |
| `apps/eval` | TypeScript | Benchmarks / evaluation harness |
| `packages/shared` | TypeScript | Cross-package constants (e.g. limits, public URLs) |
| `packages/cdp-protocol` | TypeScript | CDP typings / codegen consumed by the server |
| `packages/agent-sdk` | TypeScript | Published SDK for automation scenarios |

### Ports and environment variables

Upstream docs often cite defaults like **9100** (HTTP) and **9000** (CDP). **Your fork may use different values** (for example **9111** / **9333**) as long as **server** and **agent** configs **match**.

**Keep in sync:**

- `packages/browseros-agent/apps/server/.env.development`
- `packages/browseros-agent/apps/agent/.env.development`

**Important variables:**

| Variable | Typical role |
|----------|----------------|
| `BROWSEROS_SERVER_PORT` | Port for the Bun server (MCP, chat, health, …). |
| `BROWSEROS_CDP_PORT` | Remote debugging port for Chromium — server connects **to** this. |
| `BROWSEROS_EXTENSION_PORT` | Legacy CLI compatibility; may still be passed to launches. |
| `VITE_PUBLIC_BROWSEROS_API` | Used by the extension for auth/GraphQL/product URLs in production-like setups; in local dev, **the hostname/port must be consistent** with where the server listens. |
| `VITE_BROWSEROS_SERVER_PORT` | Used by the Vite bundle for API base URL construction in development; the stack script sets it from your env when starting. |
| `BROWSEROS_BINARY` | Path to BrowserOS/Chromium binary when not using pure stock Chrome. |

The extension resolves local server port with precedence implemented in `apps/agent/lib/browseros/helpers.ts`: prefer the port embedded in **`VITE_PUBLIC_BROWSEROS_API`**, then **`VITE_BROWSEROS_SERVER_PORT`**, then adapter defaults — so **avoid contradicting URLs** between env vars.

Copy examples:

- `apps/server/.env.example` → `apps/server/.env.development`
- `apps/agent/.env.example` → `apps/agent/.env.development`

### Development workflows

1. **Full stack (this fork):** `bun run dev` from repo root or `packages/browseros-agent` (see [Quick start](#quick-start-full-agent-stack)).

2. **Manual two-process:**  
   - Terminal A: `bun run start:agent` (under `packages/browseros-agent`)  
   - Terminal B: after CDP is up, `bun run start:server`  

3. **process-compose:** If you use [process-compose](https://github.com/F1bonacc1/process-compose), see `packages/browseros-agent/process-compose.yaml`. Note: the checked-in file may only define a subset of processes; you can extend it locally to mirror the stack script.

### Useful scripts

Run from **`packages/browseros-agent`** unless noted:

| Script | Purpose |
|--------|---------|
| `bun run dev` / `start` / `dev:stack` | Full stack via `scripts/dev/stack.ts` |
| `bun run start:server` | Server only |
| `bun run start:agent` | WXT + browser dev only |
| `bun run build` | Build pipeline as defined in this fork’s `package.json` (includes Windows-oriented server build + agent build) |
| `bun run build:agent` | Codegen + production agent build |
| `bun run lint` / `lint:fix` | Biome |
| `bun run typecheck` | TypeScript across workspaces |
| `bun run test` | Server tests (see package.json for filters) |

---

## Browser (`packages/browseros`)

This package is the **Chromium-based BrowserOS browser**: patch set, **Python** CLI (`browseros`), resources, and packaging.

### What the build system needs

- **A full Chromium checkout** at a path you provide (`chromium_src` in config, or `--chromium-src`, or `CHROMIUM_SRC` environment variable). There is **no** supported path to produce the real BrowserOS browser **without** Chromium sources — disk and time costs are significant.

### Typical CLI flow

```bash
cd packages/browseros
pip install -e .
# or: uv pip install -e .

browseros setup    # fetch/prepare Chromium (per project docs)
browseros apply    # apply patches
browseros build    # compile
browseros package  # DMG / installer / AppImage / etc.
browseros sign     # platform-specific signing
```

Pinned version files:

- `CHROMIUM_VERSION` — e.g. **146.0.7680.31** (see file for current pin)
- `BASE_COMMIT` — exact Chromium baseline

### Windows note (Python console)

When running Python CLI output that emits Unicode, set UTF-8 mode if you hit encoding errors, e.g. PowerShell:

```powershell
$env:PYTHONUTF8 = '1'
```

More detail: [`packages/browseros/README.md`](packages/browseros/README.md).

---

## Sup-agent submodule

**Sup-agent** is a **separate Git repository** linked as a submodule at:

`packages/browseros-agent/vendor/sup-agent`

### Why it exists here

- **Default skill content** for the agent is imported from paths under the submodule, e.g. `apps/server/src/skills/defaults/index.ts` imports `SKILL.md` files from  
  `vendor/sup-agent/apps/server/src/skills/defaults/...`.
- Scripts such as `packages/browseros-agent/scripts/upload-skills-catalog.ts` reference the same tree.
- Memory tooling may keep **legacy naming** compatible with Sup-agent workflows (see server `tools/memory` sources).

### Initializing and updating

After clone:

```bash
git submodule update --init --recursive
```

To move the submodule forward when you intend to track newer commits:

```bash
git submodule update --remote --merge packages/browseros-agent/vendor/sup-agent
# Then commit the updated submodule pointer in the parent repo.
```

**Canonical workflow** (independent repos, pointer commits): see **[`docs/submodule-workflow.md`](docs/submodule-workflow.md)**.

### Tests

`packages/browseros-agent/bunfig.toml` sets test `pathIgnorePatterns` to ignore `**/vendor/**` so Bun does **not** run Sup-agent’s own tests inside this monorepo; skill **files** are still loaded for BrowserOS.

---

## Documentation in this repo

| Doc | Topic |
|-----|--------|
| [`docs/submodule-workflow.md`](docs/submodule-workflow.md) | Sup-agent submodule rules |
| [`CONTRIBUTING.md`](CONTRIBUTING.md) | Upstream contribution paths (agent vs browser) |
| [`packages/browseros/README.md`](packages/browseros/README.md) | Chromium build |
| [`packages/browseros-agent/README.md`](packages/browseros-agent/README.md) | Agent monorepo |
| [`CLAUDE.md`](CLAUDE.md) (repo / packages) | Project-specific AI assistant notes |

Public product documentation for end users remains at **[docs.browseros.com](https://docs.browseros.com)**.

---

## Quality checks (Biome, Lefthook)

- **Biome** — formatting and lint (`bun run lint` in `packages/browseros-agent`).
- **Lefthook** — Git hooks (see `lefthook.yml`): Conventional Commits for commit messages, Biome on pre-commit, branch name hints on pre-push.

On **Windows**, corporate **Application Control** may block `lefthook.exe`; you may need an admin allowlist. Hooks often assume a Unix-like shell for some commands — using **Git Bash** or **WSL** for git commits can avoid edge cases.

---

## Git: fork, upstream, and submodules

This repo is suitable as a **personal or team fork** of BrowserOS.

| Remote | Typical use |
|--------|-------------|
| `origin` | Your fork (e.g. **Shimmy-Browser** on GitHub). |
| `upstream` | Optional second remote if you merge changes from another BrowserOS fork or the original project (use that project’s documented clone URL). |

Submodule URL is recorded in `.gitmodules` (Sup-agent). After pulling parent changes, run **`git submodule update --init --recursive`** so `vendor/sup-agent` matches the committed SHA.

---

## Troubleshooting

| Symptom | Likely cause | What to check |
|---------|----------------|---------------|
| “Connection failed” / Failed to fetch in Connect Apps | Server not running or **wrong port** vs extension | Same `BROWSEROS_SERVER_PORT` / API URL in agent + server env; hit `/health` on the server port. |
| Health: CDP not connected | Browser closed, wrong `BROWSEROS_CDP_PORT`, or server started before CDP | Start order: browser with debugging port **before** or **with** server; use `stack.ts` |
| Blank extension UI | Stale Vite client, uncaught React error, or unresolved imports | Re-run `bun install` in `packages/browseros-agent`; restart dev stack; check browser console |
| Vite “Failed to resolve import `@browseros/shared/...`” | Workspace deps out of date | `bun install` at `packages/browseros-agent` |
| Browser build: “chromium_src required” | No Chromium tree configured | Provide Chromium source path per `packages/browseros` README |
| `UnicodeEncodeError` in Python on Windows | Console encoding | `PYTHONUTF8=1` |

---

## License and credits

- **Shimmy-Browser** inherits licensing from upstream BrowserOS; see the **[`LICENSE`](LICENSE)** file in this repository (AGPL-3.0).
- BrowserOS credits **ungoogled-chromium** and **The Chromium Project**; see the upstream README and `LICENSE.ungoogled_chromium` where applicable.

---

## Relationship to upstream BrowserOS

Feature lists, download badges, and comparisons in the **upstream** BrowserOS README still apply at a product level. This file focuses on **this repo’s layout**, **fork-specific automation** (`bun run dev` / `stack.ts`), **Sup-agent**, and **how the browser package and agent monorepo fit together**. For this fork’s source and releases, see **[Karthikprasadm/Shimmy-Browser](https://github.com/Karthikprasadm/Shimmy-Browser)** and **[docs.browseros.com](https://docs.browseros.com)**.
