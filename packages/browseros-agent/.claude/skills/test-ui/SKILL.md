---
name: test-ui
description: Test the BrowserOS app extension UI by starting the dev environment and visually verifying changes via CDP. Covers the new tab page (left sidebar — Home, Scheduled Tasks, Settings, etc.) and the right side panel (chat interface). Use after making UI changes to apps/app/.
argument-hint: [what to test, e.g. "verify the new settings page renders correctly"]
---

# Test App UI

Visually test the BrowserOS app extension UI — both the new tab page (left sidebar) and the right side panel (chat) — by starting the dev environment and inspecting via CDP.

## When to use

After making code changes to `apps/app/` (the Chrome extension), use this skill to:
- Verify new UI components render correctly
- Check navigation between views works
- Confirm layout/styling changes look right
- Test interactive elements (buttons, inputs, forms)

## Prerequisites

- **Go** must be installed (`brew install go`) — the dev tool is written in Go
- **BrowserOS.app** must be installed at `/Applications/BrowserOS.app/`
- The `scripts/dev/inspect-ui.ts` utility must exist (CDP inspector script)

## Step 1: Start the dev environment

```bash
bun run dev:watch -- --new
```

This single command handles everything:
- Builds the Go dev CLI tool
- Picks random available ports (avoids conflicts)
- Creates a fresh browser profile
- Builds controller-ext
- Runs GraphQL codegen if `apps/app/generated/graphql/` doesn't exist
- Starts the agent extension with WXT HMR (hot module replacement)
- Waits for CDP to be ready
- Starts the MCP server

Run it in the background and **read the output to find the CDP port**:

```
[info] Ports: CDP=9552 Server=9065 Extension=9929
```

The CDP port is randomized. You MUST extract it from the output and set it for all subsequent commands:

```bash
export BROWSEROS_CDP_PORT=<port from output>
```

Wait for these messages before proceeding:
1. `[server] CDP ready`
2. `[server] HTTP server listening`

## Step 2: Discover targets

```bash
bun scripts/dev/inspect-ui.ts targets
```

You will see targets like:
- `[service_worker]` — extension background scripts (not directly testable for UI)
- `[page] chrome-extension://bflpfmnmnokmjhmgnolecpppdbdophmk/app.html#/...` — **New tab page (left sidebar)**
- `[page] sidepanel.html` — **Right side panel (chat)**

The two main testable surfaces:
- **`app.html`** — the new tab page with left sidebar (Home, Connect Apps, Scheduled Tasks, Skills, Memory, Soul, Settings)
- **`sidepanel.html`** — the right side panel chat interface

## Step 3: Navigate to the main UI

A fresh profile opens the **onboarding page** (`app.html#/onboarding`). Navigate to the home page first:

```bash
bun scripts/dev/inspect-ui.ts eval app.html "window.location.hash = '#/home'"
```

Verify with a snapshot (not screenshot — snapshot is faster and sufficient for structural checks):
```bash
bun scripts/dev/inspect-ui.ts snapshot app.html
```

## Snapshot vs Screenshot

**Prefer `snapshot` for most checks** — it's fast, text-based, and tells you what elements exist, their text, and their IDs. Use it after every navigation or interaction to verify state.

**Use `screenshot` only when you need visual verification** — layout changes, CSS/styling, colors, images, or a final "does it look right" check. Screenshots are expensive (capture → save → read image).

| Check | Use |
|-------|-----|
| Did the page navigate? | `snapshot` — look for new elements |
| Does my new component render? | `snapshot` — look for its text/role |
| Did a click change state? | `snapshot` — check element names/values |
| Is the layout correct? | `screenshot` — visual check needed |
| Do CSS changes look right? | `screenshot` — visual check needed |
| Final verification before committing | `screenshot` — one visual confirmation |

## Step 4: Test the new tab page (left sidebar)

### Get element IDs

```bash
bun scripts/dev/inspect-ui.ts snapshot app.html
```

Output shows interactive elements with IDs:
```
[52] link "Home"
[57] link "Connect Apps"
[65] link "Scheduled Tasks"
[74] link "Skills"
[103] link "Settings"
```

### Navigate via click or hash routing

**Click-based** (use element IDs from snapshot):
```bash
bun scripts/dev/inspect-ui.ts click app.html 65    # Click "Scheduled Tasks"
```

**Hash routing** (faster, no snapshot needed):
```bash
bun scripts/dev/inspect-ui.ts eval app.html "window.location.hash = '#/settings'"
bun scripts/dev/inspect-ui.ts eval app.html "window.location.hash = '#/scheduled-tasks'"
bun scripts/dev/inspect-ui.ts eval app.html "window.location.hash = '#/home'"
```

### Verify navigation

```bash
# Snapshot to confirm the page changed (fast, preferred)
bun scripts/dev/inspect-ui.ts snapshot app.html

# Screenshot only if you need to check visual layout
bun scripts/dev/inspect-ui.ts screenshot app.html /tmp/settings.png
```

### CRITICAL: Re-snapshot after every navigation

React re-renders change element IDs. **Always run snapshot again** before clicking/filling after navigating to a new view. Using stale IDs will fail.

## Step 5: Open and test the right side panel

The side panel starts **disabled** in a fresh profile. Open it using BrowserOS-specific APIs:

```bash
bun scripts/dev/inspect-ui.ts open-sidepanel
```

Wait 2 seconds for it to appear as a target, then:

```bash
bun scripts/dev/inspect-ui.ts screenshot sidepanel /tmp/panel.png
bun scripts/dev/inspect-ui.ts snapshot sidepanel
```

### Interact with the side panel

```bash
# Get element IDs
bun scripts/dev/inspect-ui.ts snapshot sidepanel
# Output: [37] textbox "What should I do?"
#         [124] button "Send"
#         [60] link "Chat history"
#         [99] button "Agent Mode ON"

# Fill the chat input and press Enter to send
bun scripts/dev/inspect-ui.ts fill sidepanel 37 "Hello world"
bun scripts/dev/inspect-ui.ts press_key sidepanel Enter

# Or click the Send button
bun scripts/dev/inspect-ui.ts click sidepanel 124

# Wait for a response to appear
bun scripts/dev/inspect-ui.ts wait_for sidepanel text "response text"

# Scroll down to see more content
bun scripts/dev/inspect-ui.ts scroll sidepanel down 3

# Hover over an element to test hover states
bun scripts/dev/inspect-ui.ts hover sidepanel 99

# Snapshot to verify state changed (fast, preferred)
bun scripts/dev/inspect-ui.ts snapshot sidepanel

# Screenshot only for visual/layout verification
bun scripts/dev/inspect-ui.ts screenshot sidepanel /tmp/result.png
```

## Step 6: Verify and iterate

### The core loop

```
snapshot → identify element IDs → click/fill/press_key → snapshot → verify
```

Use `screenshot` only when visual layout verification is needed (CSS changes, final check).

### After making code changes

1. Fix the code in `apps/app/`
2. WXT HMR will hot-reload the extension automatically (watch mode)
3. Wait 2-3 seconds for the reload to complete
4. **Re-snapshot** — element IDs WILL change after HMR reload
5. Verify the fix with snapshot (or screenshot if visual)

### Check server logs

The dev server output (running in background) contains useful diagnostics:
- `[agent]` — WXT build/HMR status, compilation errors
- `[server]` — MCP server logs, tool execution, errors
- `[build]` — Extension build output

If the UI isn't rendering, check for build errors in the `[agent]` output.

### Check for JavaScript errors

```bash
bun scripts/dev/inspect-ui.ts eval sidepanel "JSON.stringify(window.__errors || 'no errors')"
```

Or check the console for React errors:
```bash
bun scripts/dev/inspect-ui.ts eval app.html "document.querySelector('#root')?.innerHTML?.substring(0, 200)"
```

### Verify API connectivity

The extension talks to the MCP server. Verify the server is reachable:
```bash
bun scripts/dev/inspect-ui.ts eval sidepanel "fetch('http://127.0.0.1:<serverPort>/health').then(r => r.ok).catch(() => false)"
```

### Common issues

| Symptom | Cause | Fix |
|---------|-------|-----|
| Blank page after navigation | React render error | Check `eval` for JS errors |
| Element IDs don't match | Page re-rendered (HMR/navigation) | Re-run `snapshot` before interacting |
| `open-sidepanel` fails | Extension not fully loaded | Wait longer after dev server starts |
| Click does nothing | Element not visible (below fold) | Use `scroll` first, then re-snapshot |
| `wait_for` times out | Content hasn't loaded yet | Check server logs for API errors |

## Available commands reference

| Command | Description |
|---------|-------------|
| `targets` | List all CDP targets, marks extension pages with `[EXTENSION]` |
| `screenshot <target> [file]` | Capture PNG screenshot (default: `screenshot.png`) |
| `snapshot <target>` | Print accessibility tree with `[elementId] role "name"` |
| `click <target> <elementId>` | Click element by ID (3-tier coordinate fallback + JS click) |
| `fill <target> <elementId> <text>` | Focus element, clear, type text |
| `press_key <target> <key>` | Press key or combo: `Enter`, `Escape`, `Tab`, `Control+A`, `Meta+Shift+P` |
| `scroll <target> <dir> [amount]` | Scroll `up`/`down`/`left`/`right`, amount in ticks (default 3) |
| `hover <target> <elementId>` | Hover over element (for tooltips, hover states) |
| `select_option <target> <id> <val>` | Select dropdown option by value or visible text |
| `wait_for <target> text\|selector <v>` | Wait up to 10s for text or CSS selector to appear |
| `eval <target> <expression>` | Run JavaScript in the target's context |
| `open-sidepanel` | Enable and open the right side panel |

`<target>` is a URL substring (e.g., `sidepanel`, `app.html`) or numeric index from `targets` output.

## Known app.html routes

These can be used with `eval app.html "window.location.hash = '#/<route>'"`:

| Route | View |
|-------|------|
| `/home` | Home page with search bar and top sites |
| `/settings` | Settings (LLM providers, customization, workflows, MCP) |
| `/scheduled-tasks` | Scheduled Tasks management |
| `/onboarding` | Onboarding flow (first-run experience) |

## Gotchas learned from real testing

1. **Ports are randomized** with `--new` — always extract from dev server output
2. **Fresh profile = onboarding page** — navigate to `#/home` to see the main UI
3. **Element IDs change after navigation** — always re-snapshot before clicking
4. **Side panel starts disabled** — `open-sidepanel` handles the BrowserOS-specific enable + toggle API
5. **`Input.enable` does not exist** — the CDP Input domain has no enable method (already handled in the script)
6. **`DOM.getDocument` required** — must be called before DOM operations like `pushNodesByBackendIdsToFrontend` (already handled in the script)
7. **Settings sub-navigation** — the settings page has its own left sidebar (BrowserOS AI, Chat & Council Provider, Search Provider, Customize BrowserOS, BrowserOS as MCP, Workflows) — use snapshot + click to navigate within settings
