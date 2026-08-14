# Experiment: agent-first window

The reverse of today's BrowserOS layout. Instead of the agent living in a side
panel of the browser, **the agent is the window** and the browser docks to the
right behind a toggle — the shape ChatGPT's and Codex's in-app browsers use.

| State | What the user sees |
|---|---|
| `AGENT_ONLY` (default at launch) | The agent fills the whole window. No tab strip, no toolbar. macOS traffic lights and window dragging intact. |
| `SPLIT` | Agent pinned left at 40% (min 320px, resizable). Real Chromium on the right: tab strip, omnibox, live contents. |

Toggling is instant and **open tabs survive the round trip**.

## ⚠️ Read this before touching the main patch set

**These patches are generated against Chromium `137.0.7232.69`**, from the local
BrowserOS stack at `670286bc0e361`. The main `chromium_patches/` set targets
**151**. That is why this lives in `experiments/` and not in there — dropping
these files into `chromium_patches/` would overwrite ten existing 151 patches for
the same source files.

To land this properly: re-apply the change on a 151 tree and re-extract with the
normal patch tooling. Treat what is here as the reference implementation, not as
something to copy in.

## How it works

A new `AgentPaneView` is a direct child of `BrowserView`. In
`BrowserViewLayout::Layout()` the `vertical_layout_rect_` is inset from the left
by the agent width **before any other child is laid out** — after which the tab
strip, toolbar, bookmark bar, infobars, contents and the ordinary side panel all
lay themselves out inside the remaining rectangle, unchanged. That one inset is
the feature.

The side panel was deliberately **not** reused: it is hard-capped at 2/3 of
contents width, is laid out below the toolbar, and carries a header, a 16px
border and a rounded-corner overlay. Three independent designs reached that
conclusion separately.

Four call sites break when `vertical_layout_rect_.x() != 0` and are fixed here:
`UpdateTopContainerBounds()` building its rect at origin; children of
`top_container_` being positioned in *its* coordinate space (needs a runtime
parent check, because `bookmark_bar_` reparents at runtime); the macOS
caption-button inset stealing width from the tab strip in SPLIT; and
`GetMinimumSize()` not accounting for the pane.

## Layout

```
chromium_patches/          one patch per source file, mirroring the Chromium tree
                           (same convention as packages/browseros/chromium_patches)
features.yaml              the two features and their file lists
combined-milestone1-2.patch        15 files — the window + the hosted agent pane
combined-milestone3-js-toggle.patch 6 files — the chrome.browserOS JS API
extension-chromium137-guards.patch  agent extension fix, `git am` format
```

The two `combined-*.patch` files are plain `git diff` output and apply directly
to a 137 tree; the per-file patches under `chromium_patches/` are the same
content in BrowserOS's extraction format.

## What is in each feature

**`agent-first-window`** — `AgentPaneView` and its layout integration, the
`IDC_TOGGLE_AGENT_SPLIT` command and `⌘⇧E` accelerator, the macOS titlebar and
hit-testing work, and hosting the agent extension page in the pane via
`ExtensionViewHostFactory::CreateSidePanelHost()` with a placeholder label
fallback when the extension is absent.

**`agent-first-window-js-toggle`** — `chrome.browserOS.toggleAgentSplit()`, so a
button inside the agent's own UI can open the browser. It resolves the *sender's*
`WebContents` to its owning `BrowserView`, so it cannot toggle the wrong window,
and fails closed when there is no owner.

**`extension-chromium137-guards.patch`** — separate from the C++: the agent
extension called `chrome.sidePanel` APIs that 137 does not expose, so its service
worker threw on startup. Feature-detection guards plus tests. Useful on its own,
independent of this experiment.

## Verified behaviour

Logged from a running binary in a 1200×1241 window:

```text
AGENT_ONLY  agent=0,0 1200x1241  contents=0x0 hidden
SPLIT       agent=0,0 480x1241   top=480,0 720x87   contents=480,87 720x1154
```

In SPLIT the agent takes the left 480px (40%) and **both** the top chrome and the
web contents start at x=480 — the whole browser moved right.

Tab survival, across a full collapse and expand, by raw `WebContents` pointer:

```text
BEFORE_SPLIT       tabs=3 visible=1   0x3f8008200 0x3d083a000 0x3d08b1000
DURING_AGENT_ONLY  tabs=3 visible=0   0x3f8008200 0x3d083a000 0x3d08b1000
AFTER_SPLIT        tabs=3 visible=1   0x3f8008200 0x3d083a000 0x3d08b1000
```

Identical pointers: the objects were never destroyed or recreated, so the collapse
is a visibility change and not a teardown.

The JS toggle, evaluated in the agent page:

```text
> await chrome.browserOS.toggleAgentSplit()
M3JSToggle  before=AGENT_ONLY  after=SPLIT
```

## Running it

The agent pane renders a placeholder unless the extension is loaded, and the
built-in loader must stand down or it collides with the unpacked copy over the
same extension id:

```bash
"<out>/BrowserOS Dev.app/Contents/MacOS/BrowserOS Dev" \
  --user-data-dir=/tmp/bos-demo --profile-directory=Default \
  --no-first-run --no-default-browser-check \
  --disable-browseros-extensions \
  --load-extension=<path to built agent extension>
```

`--disable-browseros-extensions` is required because this change adds the agent id
to `kBrowserOSExtensions`, so the external loader and `--load-extension` both claim
it. The proper fix is for the loader to yield to an unpacked extension with the
same id; that is not done here.

## Not done

No prefs or persistence (every window starts agent-first, split fixed at 40%). No
feature flag. `⌘⇧E` is wired in both accelerator tables but a physical keypress was
never tested. Divider dragging, RTL, fullscreen transitions and the
find-bar/download-shelf/modal collapse guards are untested by hand. The
`unit_tests` target does not build, but that is pre-existing and unrelated — it
dies in generated Settings TypeScript before reaching any layout test.
