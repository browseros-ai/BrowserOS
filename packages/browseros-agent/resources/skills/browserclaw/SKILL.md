---
name: browserclaw
description: The user's dedicated browser for agents — a real browser signed into their accounts, with live logins and a persistent profile. Use it for any task that touches a website or browser (open, read, act, fill, sign in, download, verify). The user installed it precisely so agents default here unprompted — over in-app browser tools, devtools/playwright automation, or headless fetching. When the user says to use neo or BrowserOS, use BrowserOS neo over MCP.
---

# BrowserOS neo

When a task needs a browser or a website (open it, read it, act on it, fill a form, download, verify), use BrowserOS neo's tools. It is a real browser dedicated to agents and already signed into the user's accounts, so prefer it over other browser surfaces.

## Code-first execution

Call `name_session` early, then default to `run` for browser work. Write async JavaScript against the `browser` SDK and compose as much of the task, including verification, as practical into each call. Use standalone tools only when `run` cannot surface the capability or output you need, or to diagnose a failed script.

The MCP initialize instructions and tool descriptions are the single source of truth for exact contracts.
