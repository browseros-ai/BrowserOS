# How to run Shimmy-Browser (agent + browser) and push to GitHub

This guide walks through **starting the full development stack** (extension UI + Chromium dev browser + Bun server), optional pieces, and **publishing changes to GitHub**, including the **Sup-agent** submodule.

**To run the project, go to [Run the project (copy-paste commands)](#run-the-project-copy-paste-commands) first**, then use the rest of the doc for details.

---

## Table of contents

1. [Run the project (copy-paste commands)](#run-the-project-copy-paste-commands)
2. [What gets started](#what-gets-started)
3. [Prerequisites](#prerequisites)
4. [Clone the repository](#clone-the-repository)
5. [Install dependencies](#install-dependencies)
6. [Configure environment (required)](#configure-environment-required)
7. [Start everything (recommended)](#start-everything-recommended)
8. [Alternative: start agent and server separately](#alternative-start-agent-and-server-separately)
9. [Verify the stack is healthy](#verify-the-stack-is-healthy)
10. [VS Code](#vs-code)
11. [What counts as “the browser” in dev](#what-counts-as-the-browser-in-dev)
12. [Optional: build the real BrowserOS Chromium](#optional-build-the-real-browseros-chromium)
13. [Stop the stack](#stop-the-stack)
14. [Push your work to GitHub](#push-your-work-to-github)
15. [Submodule (Sup-agent) and GitHub](#submodule-sup-agent-and-github)
16. [Troubleshooting](#troubleshooting)

---

## Run the project (copy-paste commands)

These are the **exact commands** to go from a fresh clone to a running **browser + extension + server**. Adjust the clone URL to your fork.

### One-time setup (first time on this machine)

**Bash (macOS, Linux, Git Bash on Windows):**

```bash
git clone https://github.com/<your-user>/Shimmy-Browser.git
cd Shimmy-Browser
git submodule update --init --recursive
cd packages/browseros-agent
bun install
cp apps/server/.env.example apps/server/.env.development
cp apps/agent/.env.example apps/agent/.env.development
```

**PowerShell (Windows)** — if your repo is at `D:\Shimmy-Browser`, run:

```powershell
cd D:\Shimmy-Browser
git submodule update --init --recursive
cd packages\browseros-agent
bun install
Copy-Item apps\server\.env.example apps\server\.env.development
Copy-Item apps\agent\.env.example apps\agent\.env.development
```

Edit **`packages/browseros-agent/apps/server/.env.development`** and **`packages/browseros-agent/apps/agent/.env.development`**: set the **same** `BROWSEROS_CDP_PORT` and `BROWSEROS_SERVER_PORT` in both. For local API calls, set `VITE_PUBLIC_BROWSEROS_API` in the **agent** file to your server URL (for example `http://127.0.0.1:9111` if the server uses port `9111`). See [Configure environment (required)](#configure-environment-required).

### Start the full stack (every day)

From the **repository root** (`Shimmy-Browser`, e.g. `D:\Shimmy-Browser`):

```bash
bun run dev
```

**PowerShell:**

```powershell
cd D:\Shimmy-Browser
bun run dev
```

This runs the stack script: WXT + Chromium + extension, waits for CDP, then starts the Bun server. **`bun run start`** at the repo root is the same as **`bun run dev`**.

### Same stack without npm scripts (explicit script path)

From the repo root:

```bash
bun packages/browseros-agent/scripts/dev/stack.ts
```

### Start from `packages/browseros-agent` only

```bash
cd packages/browseros-agent
bun run dev
```

Equivalent: **`bun run start`** or **`bun run dev:stack`** (all three call `scripts/dev/stack.ts`).

### Two terminals (manual order)

**Terminal 1** — from `packages/browseros-agent`:

```bash
cd packages/browseros-agent
bun run start:agent
```

**Terminal 2** — after Chromium is up and CDP is listening:

```bash
cd packages/browseros-agent
bun run start:server
```

### Command cheat sheet (run-related)

| Command | Working directory | Effect |
|---------|-------------------|--------|
| `bun run dev` | Repo root **or** `packages/browseros-agent` | **Recommended:** agent + browser, then server (full stack) |
| `bun run start` | Same as above | Same as `bun run dev` (root: `package.json`; agent: maps to stack) |
| `bun packages/browseros-agent/scripts/dev/stack.ts` | Repo root | Same full stack, explicit path |
| `bun run start:agent` | `packages/browseros-agent` | WXT dev + Chromium + extension only |
| `bun run start:server` | `packages/browseros-agent` | Bun MCP server only (use after CDP is up) |
| `bun run --filter @browseros/agent dev` | `packages/browseros-agent` | Same as `start:agent` (underlying command) |
| `bun run --filter @browseros/server start` | `packages/browseros-agent` | Same as `start:server` (underlying command) |

---

## What gets started

In local development you normally run:

| Piece | What it is |
|--------|------------|
| **Agent (extension)** | `packages/browseros-agent/apps/agent` — WXT + Vite + React. Dev mode launches **Chromium** with the extension loaded (this is your “browser” during development). |
| **Server** | `packages/browseros-agent/apps/server` — **Bun** HTTP server: chat, MCP tools, health, CDP client to control the browser. |

The helper script **`packages/browseros-agent/scripts/dev/stack.ts`**:

1. Loads **`packages/browseros-agent/apps/agent/.env.development`**.
2. Starts **agent dev** (`bun run --filter @browseros/agent dev`) → browser + extension.
3. Waits until **CDP** answers at `http://127.0.0.1:<BROWSEROS_CDP_PORT>/json/version`.
4. Starts the **server** (`bun run --filter @browseros/server start`).

So one command brings up **browser + agent UI + server** in the right order.

---

## Prerequisites

| Tool | Why |
|------|-----|
| **[Bun](https://bun.sh)** | The monorepo uses Bun workspaces and scripts (`package.json` specifies a `packageManager`; install a compatible Bun version). |
| **Git** | Clone, submodules, push to GitHub. |
| **GitHub account** | Push remote (SSH or HTTPS). |

You do **not** need to build the full Chromium fork just to run the agent stack.

---

## Clone the repository

**HTTPS:**

```bash
git clone https://github.com/<your-user>/Shimmy-Browser.git
cd Shimmy-Browser
```

**SSH:**

```bash
git clone git@github.com:<your-user>/Shimmy-Browser.git
cd Shimmy-Browser
```

### Initialize the Sup-agent submodule

The repo pins **`packages/browseros-agent/vendor/sup-agent`**. After clone:

```bash
git submodule update --init --recursive
```

If you skip this, paths that load default skills from the submodule may be missing until you init.

---

## Install dependencies

All agent/server/extension installs happen from the **agent monorepo** directory:

```bash
cd packages/browseros-agent
bun install
```

`postinstall` may build shared artifacts (for example **agent-sdk**). Let it finish.

---

## Configure environment (required)

The stack script reads **`packages/browseros-agent/apps/agent/.env.development`** only for loading env into the spawned processes, but the **server** must use the **same** CDP and HTTP ports. Maintain **two** files and keep ports aligned:

1. `packages/browseros-agent/apps/agent/.env.example` → copy to **`apps/agent/.env.development`**
2. `packages/browseros-agent/apps/server/.env.example` → copy to **`apps/server/.env.development`**

**PowerShell (from `packages/browseros-agent`):**

```powershell
Copy-Item apps\server\.env.example apps\server\.env.development
Copy-Item apps\agent\.env.example apps\agent\.env.development
```

**bash:**

```bash
cp apps/server/.env.example apps/server/.env.development
cp apps/agent/.env.example apps/agent/.env.development
```

### Ports you must keep in sync

Set the **same** values in **both** files:

| Variable | Meaning |
|----------|---------|
| `BROWSEROS_CDP_PORT` | Remote-debugging port for Chromium. The **server connects here** (CDP client). |
| `BROWSEROS_SERVER_PORT` | HTTP port for the Bun server (chat, MCP, `/health`, etc.). |
| `BROWSEROS_EXTENSION_PORT` | Legacy compatibility; keep consistent if present in examples. |

In **`apps/agent/.env.development`** also set:

| Variable | Meaning |
|----------|---------|
| `VITE_BROWSEROS_SERVER_PORT` | Must match `BROWSEROS_SERVER_PORT` for local API calls (the stack script overwrites this when using `stack.ts`, but your file should still be consistent). |
| `VITE_PUBLIC_BROWSEROS_API` | For production-like API hosts; for **local dev**, use a URL whose **host and port** match your server (for example `http://127.0.0.1:9111`) so helpers do not point at the wrong port. |

**Defaults inside `scripts/dev/stack.ts`** if variables are missing or invalid:

- `BROWSEROS_CDP_PORT` → **9333**
- `BROWSEROS_SERVER_PORT` → **9111**

Pick any free ports on your machine; just keep **agent** and **server** files matching.

### Browser binary (optional)

`apps/agent/.env.development` may set:

- `BROWSEROS_BINARY` — path to **BrowserOS.app** / **BrowserOS.exe** / Chromium if you want a specific binary instead of the dev default.

If unset, dev tooling uses its normal Chromium selection (per WXT / web-ext config).

---

## Start everything (recommended)

### From the repository root

```bash
bun run dev
```

(`bun run start` is the same script.)

This runs:

`packages/browseros-agent/scripts/dev/stack.ts`

### From `packages/browseros-agent`

```bash
cd packages/browseros-agent
bun run dev
```

Equivalent script names: `dev`, `start`, `dev:stack`.

**First run** can take a while while Vite/WXT compiles and Chromium starts.

---

## Alternative: start agent and server separately

Use this if you prefer two terminals or are debugging one side.

**Terminal 1 — agent + browser only** (from `packages/browseros-agent`):

```bash
bun run start:agent
```

**Terminal 2 — server** (after the browser is up and CDP is listening):

```bash
bun run start:server
```

Order matters: the server expects to attach to CDP, so the browser should be running with the configured **`BROWSEROS_CDP_PORT`**.

---

## Verify the stack is healthy

Replace `<SERVER_PORT>` and `<CDP_PORT>` with your values (for example **9111** and **9333**).

**Server health:**

```bash
curl -s http://127.0.0.1:<SERVER_PORT>/health
```

You want a JSON body that indicates the server is up and CDP connectivity matches expectations (for example `cdpConnected` where applicable).

**CDP (browser debugging endpoint):**

```bash
curl -s http://127.0.0.1:<CDP_PORT>/json/version
```

You should get JSON describing the browser.

---

## VS Code

The workspace includes **`.vscode/tasks.json`** with:

- **Task:** `Dev: full stack (agent + browser + server)`  
- **Command:** `bun run dev` at the workspace folder  

Use **Terminal → Run Task…** to start the same stack as the CLI.

---

## What counts as “the browser” in dev

- **Development:** WXT/Web-ext starts **Chromium** with the unpacked extension. That process exposes **CDP** on `BROWSEROS_CDP_PORT`. This is the browser you use while coding.
- **Production-style:** A packaged **BrowserOS** build from `packages/browseros` (custom Chromium) — heavy setup; not required for everyday agent development.

---

## Optional: build the real BrowserOS Chromium

Only if you need the **custom Chromium fork**:

1. See **`packages/browseros/README.md`**.
2. You need a full **Chromium source tree** and a lot of disk space (~100 GB class).
3. Use the Python CLI (`browseros setup`, `apply`, `build`, …).

On Windows, if Python prints **UnicodeEncodeError** in the console, try:

```powershell
$env:PYTHONUTF8 = '1'
```

before running Python.

---

## Stop the stack

In the terminal where `bun run dev` is running:

- **Ctrl+C** — the stack script kills both the agent process and the server process.

If something is stuck, close the Chromium window launched by dev, then stop the terminal process.

---

## Push your work to GitHub

### 1. Check status

```bash
git status
```

### 2. Stage changes

```bash
git add -p          # interactive, recommended for review
# or
git add <paths>
```

### 3. Commit

This repo uses **Lefthook** with **Conventional Commits** for commit messages (see `lefthook.yml`). Prefer:

```text
<type>(<scope>): <short description>
```

Examples: `feat(agent): improve startup`, `fix(server): align CDP port`, `docs: update how_to_run`.

```bash
git commit -m "feat(agent): describe your change"
```

If **lefthook** or **Biome** is blocked on Windows (security software), you may need to allow **`lefthook.exe`** or run hooks from an environment where they are permitted.

### 4. Set remote (first time or when fixing)

List remotes:

```bash
git remote -v
```

Add your GitHub fork as **`origin`** if needed:

```bash
git remote add origin https://github.com/<your-user>/Shimmy-Browser.git
# or SSH:
git remote add origin git@github.com:<your-user>/Shimmy-Browser.git
```

Optional **upstream** (original BrowserOS) for merges:

```bash
git remote add upstream https://github.com/browseros-ai/BrowserOS.git
```

### 5. Push

```bash
git push -u origin main
```

Use your real branch name instead of `main` if different (`master`, `develop`, etc.).

---

## Submodule (Sup-agent) and GitHub

`packages/browseros-agent/vendor/sup-agent` is a **separate repo**. Changes inside it are **not** pushed when you only push the parent.

### Working only in the parent repo

No submodule edits:

```bash
git push origin <branch>
```

### You changed files inside `vendor/sup-agent`

1. **Commit and push inside the submodule** (Sup-agent’s own remote):

   ```bash
   cd packages/browseros-agent/vendor/sup-agent
   git status
   git add ...
   git commit -m "feat: your sup-agent change"
   git push origin <branch>
   ```

2. **Return to the repo root** and commit the **updated submodule pointer**:

   ```bash
   cd ../../../../..
   git add packages/browseros-agent/vendor/sup-agent
   git commit -m "chore: bump sup-agent submodule"
   git push origin <branch>
   ```

More detail: **`docs/submodule-workflow.md`**.

After **cloning** or **pulling** on another machine:

```bash
git submodule update --init --recursive
```

---

## Troubleshooting

| Problem | What to do |
|---------|------------|
| **Connect failed / Failed to fetch** in the UI | Server not running, or **`BROWSEROS_SERVER_PORT`** / **`VITE_PUBLIC_BROWSEROS_API`** disagree between agent and server. Fix `.env.development` files and restart. |
| **`/health` bad or CDP disconnected** | Browser closed before server, or wrong **`BROWSEROS_CDP_PORT`**. Start browser first or use **`bun run dev`**. |
| **Blank extension UI** | Restart dev stack; run **`bun install`** again under `packages/browseros-agent`; check the browser console for errors. |
| **Import errors (`@browseros/shared`, …)** | Dependencies out of sync — **`bun install`** in `packages/browseros-agent`. |
| **Submodule folder empty** | Run **`git submodule update --init --recursive`**. |
| **Commit rejected by hook** | Use **Conventional Commits** format; fix **Biome** issues with **`bun run lint:fix`** in `packages/browseros-agent` where applicable. |

---

## Quick reference

| Goal | Command |
|------|---------|
| Submodule init | `git submodule update --init --recursive` |
| Install | `cd packages/browseros-agent && bun install` |
| Copy env files | `cp apps/server/.env.example apps/server/.env.development` and same for `apps/agent/` (PowerShell: `Copy-Item …`) |
| **Full stack (browser + agent UI + server)** | From repo root: **`bun run dev`** or **`bun run start`** |
| Full stack (explicit) | `bun packages/browseros-agent/scripts/dev/stack.ts` |
| Agent + browser only | `cd packages/browseros-agent && bun run start:agent` |
| Server only | `cd packages/browseros-agent && bun run start:server` |
| Stop | **Ctrl+C** in the terminal running the stack |
| Push | `git push -u origin <branch>` |

For architecture and repo map, see **`README.md`**.
