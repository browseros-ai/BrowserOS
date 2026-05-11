# Linux Manual Start Reference

Use this only when the standard dev command fails on Linux.

## Symptoms

- `./tools/dev/run.sh watch --manual` fails because Lima is missing
- BrowserOS AppImage needs explicit flags
- Server must be started separately from the BrowserOS app

## Verified fallback

### 1. Kill old runtime

```bash
REPO_ROOT="${REPO_ROOT:-$(cd "$(dirname "$0")/../../.." 2>/dev/null && pwd || pwd)}"
cd "$REPO_ROOT"
pkill -9 -f '\.browseros-dev-chrome' 2>/dev/null || true
pkill -9 -f 'apps/server/src/index.ts' 2>/dev/null || true
```

### 2. Rebuild dev extension

```bash
cd "$REPO_ROOT/packages/browseros-agent"
bun run build:agent:dev
```

### 3. Start BrowserOS AppImage with the local dev extension

```bash
cd "$REPO_ROOT/packages/browseros-agent"
APP="${BROWSEROS_APP_PATH:-$HOME/Downloads/alta/BrowserOS.AppImage}"
EXT_DIR="$REPO_ROOT/packages/browseros-agent/apps/agent/dist/chrome-mv3-dev"
nohup "$APP" \
  --no-sandbox \
  --no-first-run \
  --no-default-browser-check \
  --use-mock-keychain \
  --show-component-extension-options \
  --disable-browseros-server \
  --disable-browseros-extensions \
  --remote-debugging-port=9010 \
  --browseros-mcp-port=9110 \
  --browseros-server-port=9110 \
  --user-data-dir="$HOME/.browseros-dev-chrome" \
  --load-extension="$EXT_DIR" \
  chrome://newtab \
  > /tmp/browseros-dev-browser.log 2>&1 &
```

### 4. Start the server separately

```bash
cd "$REPO_ROOT/packages/browseros-agent"
nohup env \
  BROWSEROS_SKIP_OPENCLAW=1 \
  BROWSEROS_CDP_PORT=9010 \
  BROWSEROS_SERVER_PORT=9110 \
  BROWSEROS_EXTENSION_PORT=9305 \
  bun run --filter @browseros/server start:ci \
  > /tmp/browseros-dev-server.log 2>&1 &
```

## Verification

### Check logs

- browser: `/tmp/browseros-dev-browser.log`
- server: `/tmp/browseros-dev-server.log`

### Check health

Open:

- `http://127.0.0.1:9110/health`

Expected:

```json
{"status":"ok","cdpConnected":true}
```

## Why this fallback exists

On hosts where the standard watch/manual launcher depends on Lima and does not complete, use this fallback.

**Setup:** Set `REPO_ROOT` to the BrowserOS checkout root (or run from that directory). Set `BROWSEROS_APP_PATH` to override the default AppImage location (`$HOME/Downloads/alta/BrowserOS.AppImage`).
