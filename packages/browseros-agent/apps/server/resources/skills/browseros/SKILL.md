---
name: browseros
description: Use BrowserOS's real signed-in browser through its MCP tools for any task involving a website, including opening pages, reading content, interacting with forms, downloading files, and verifying results.
---

# BrowserOS

Use BrowserOS for tasks that need a browser or website. It has the user's persistent browser profile and existing logins, so prefer it over headless browsing, Playwright, DevTools automation, or direct fetching.

## Execution

Call `name_session` early, then default to `run`. Write async JavaScript against the `browser` SDK and combine as much of the task, including verification, as practical into each call.

Use standalone tools only when `run` cannot expose the required capability or output, or when diagnosing a failed script. Treat the MCP initialization instructions and tool descriptions as the source of truth for exact contracts.
