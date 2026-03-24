# How To Run Project From Code

This guide explains how to run the BrowserOS project from source code, especially on Windows.

## What "run from code" means here

- The browser app binary itself is Chromium-based and heavy to build.
- In day-to-day development, you typically:
  - run the agent server from code (`apps/server`)
  - run the agent/controller extensions from code (`apps/agent`, `apps/controller-ext`)
  - connect them to a BrowserOS/Chromium binary installed locally

This is the fastest and most reliable workflow for customizing browser behavior + agent behavior.

## Prerequisites

- Windows 10/11
- [Bun](https://bun.sh) installed
- Git installed
- BrowserOS Windows binary installed (or Chromium binary at minimum)
  - Example BrowserOS installer: `BrowserOS_installer.exe`

## 1) Install dependencies

From repo root:

```powershell
cd D:\Shimmy-Browser\packages\browseros-agent
bun install
```

## 2) Create env files

Create server env:

```powershell
Copy-Item "apps\server\.env.example" "apps\server\.env.development"
```

Create agent env:

```powershell
Copy-Item "apps\agent\.env.example" "apps\agent\.env.development"
```

## 3) Configure local dev ports and paths

### `apps/server/.env.development`

Use these values:

```env
BROWSEROS_CDP_PORT=9100
BROWSEROS_SERVER_PORT=9101
BROWSEROS_EXTENSION_PORT=9301
```

### `apps/agent/.env.development`

Use these values:

```env
BROWSEROS_CDP_PORT=9100
BROWSEROS_SERVER_PORT=9101
BROWSEROS_EXTENSION_PORT=9301
VITE_BROWSEROS_SERVER_PORT=9101
VITE_PUBLIC_BROWSEROS_API=http://127.0.0.1:9101
GRAPHQL_SCHEMA_PATH=D:\Shimmy-Browser\packages\browseros-agent\apps\agent\schema\schema.graphql
```

Set browser binary path (important on Windows):

```env
BROWSEROS_BINARY=C:\Users\<YOUR_USER>\AppData\Local\Chromium\Application\chrome.exe
```

If you have BrowserOS executable elsewhere, use that path instead.

## 4) Start Browser/Chromium with CDP enabled

If not already started by `start:agent`, launch manually:

```powershell
Start-Process "C:\Users\<YOUR_USER>\AppData\Local\Chromium\Application\chrome.exe" -ArgumentList "--remote-debugging-port=9100","--remote-debugging-address=127.0.0.1","about:blank"
```

Verify CDP:

```powershell
Invoke-WebRequest -UseBasicParsing http://localhost:9100/json/version
```

You should get HTTP 200.

## 5) Start server from code

Terminal A:

```powershell
cd D:\Shimmy-Browser\packages\browseros-agent
bun run --filter @browseros/server start
```

Expected:

- HTTP server on `http://127.0.0.1:9101`
- Controller WS on `ws://127.0.0.1:9301`

Health check:

```powershell
Invoke-WebRequest -UseBasicParsing http://127.0.0.1:9101/health
```

## 6) Start agent/controller extensions from code

Terminal B:

```powershell
cd D:\Shimmy-Browser\packages\browseros-agent
bun run start:agent
```

Expected output includes:

- WXT dev server started
- extension build completed
- browser opened

At this point, BrowserOS agent UI should be available in the opened browser window (new tab / side panel / extension action).

## 7) Verify everything is connected

- CDP: `http://localhost:9100/json/version` -> 200
- Server: `http://127.0.0.1:9101/health` -> 200
- MCP: `http://127.0.0.1:9101/mcp` -> 200
- Server logs show extension/controller connected

## Common issues and quick fixes

### 1) `Failed to start CDP on port ...`

- Browser not exposing CDP on that port.
- Fix:
  - start browser with `--remote-debugging-port=<same-port>`
  - ensure `.env` `BROWSEROS_CDP_PORT` matches.

### 2) `Invalid URL` in `apps/agent/wxt.config.ts`

- `VITE_PUBLIC_BROWSEROS_API` missing/invalid.
- Fix: set `VITE_PUBLIC_BROWSEROS_API=http://127.0.0.1:9101`.

### 3) `No schema found` during `graphql-codegen`

- `GRAPHQL_SCHEMA_PATH` empty or wrong.
- Fix: point to `apps/agent/schema/schema.graphql`.

### 4) Tries macOS binary path `/Applications/BrowserOS.app/...`

- `BROWSEROS_BINARY` not set for Windows.
- Fix: set `BROWSEROS_BINARY` to your local browser executable.

### 5) `port 9300 in use`

- Controller port conflict.
- Fix: use `BROWSEROS_EXTENSION_PORT=9301` in both server and agent env files.

## Where to customize

- Agent server logic/tools:
  - `packages/browseros-agent/apps/server/src`
- Agent UI:
  - `packages/browseros-agent/apps/agent`
- Controller extension:
  - `packages/browseros-agent/apps/controller-ext`
- Shared constants:
  - `packages/browseros-agent/packages/shared/src/constants`

## Optional: build browser binary from source (advanced)

If you need to modify Chromium-level browser code, use build scripts in:

- `packages/browseros/build/`
- config files in `packages/browseros/build/config/`

This path is much heavier (time + disk) than the code-driven server/extension workflow above.

