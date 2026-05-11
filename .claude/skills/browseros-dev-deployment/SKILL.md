---
name: browseros-dev-deployment
description: Use when rebuilding, restarting, or verifying the local BrowserOS development app/server workflow, especially after UI or extension changes that must be reloaded into the dev app
---

# BrowserOS Dev Deployment

## Overview

Use the project mise tasks as the single entry point for local BrowserOS dev rebuild/restart work.

**Core principle:** one command → deterministic rebuild/restart/health verification.

## When to Use

- BrowserOS UI changes are not showing up
- The dev app needs a clean restart
- You changed extension UI/settings/routes/assets
- You need the exact local dev restart workflow

Do **not** use this for production releases.

## Quick Workflow

### Preferred shortcut

```bash
cd <repo-root>            # this repo's root directory
mise trust .mise.toml   # first run only
mise run browseros:restart-dev
```

### Other shortcuts

```bash
mise run browseros:kill-dev
mise run browseros:build-dev
mise run browseros:start-dev
mise run browseros:health-dev
```

If the standard dev path is blocked on the current platform, load `linux-manual-start.md` in this skill directory. The project mise tasks codify that verified Linux fallback.

### Verify before done

Minimum checks:

- BrowserOS app is open
- `mise run browseros:health-dev` returns `{"status":"ok","cdpConnected":true}`
- the changed UI surface is visible in BrowserOS

## Quick Reference

| Intent | Command |
|---|---|
| Full restart | `mise run browseros:restart-dev` |
| Kill old runtime | `mise run browseros:kill-dev` |
| Rebuild dev bundle | `mise run browseros:build-dev` |
| Start app + server | `mise run browseros:start-dev` |
| Health check | `mise run browseros:health-dev` |

## Common Mistakes

- Rebuilding without killing the old dev profile first
- Checking `chrome://settings` instead of BrowserOS app settings
- Claiming restart success without a `/health` check
- Assuming the standard watch flow works on every host

## Notes

- Advanced Config is inside BrowserOS app settings, not browser native settings.
- If the BrowserOS window opens but changes are missing, assume stale dev runtime first.
