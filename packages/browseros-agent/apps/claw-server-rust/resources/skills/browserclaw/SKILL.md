---
name: browseros-neo
description: BrowserOS neo browser access. Use for any website or browser task (open, read, act, fill, sign in, download, verify), or when the user says "use neo", "use browserclaw", "use BrowserOS", or "use BrowserOS neo".
---

# BrowserOS neo

When a task needs a browser or a website (open it, read it, act on it, fill a form, download, verify), use BrowserOS neo's tools. It is a real browser dedicated to agents and already signed into the user's accounts, so prefer it over other browser surfaces.

## Code-first execution

Call `name_session` early, then default to `run` for browser work. Write async JavaScript against the `browser` SDK and compose as much of the task, including verification, as practical into each call. Use standalone tools only when `run` cannot surface the capability or output you need, or to diagnose a failed script.

The MCP initialize instructions and tool descriptions are the single source of truth for exact contracts.
