# Neo SDK

The hosted Neo implementation lives here. This is a grouping folder, not a Cargo or npm
package: each child owns its manifest, and the agent workspace lists the eight Rust members
explicitly. Keep tool configuration inside the packages that need it so it cannot change
the lint or formatting rules of sibling crates.

Give agents [agent.d.ts](agent.d.ts) with the MCP URL for the task API, script globals, and a four-run example.

Each `run` starts with fresh JavaScript. With no task, built-in `page` and
`context.newPage()` each open a new tab with no task, never an existing tab.
To keep tabs across runs, create a task with `await neo.createTask(name)`, return `task.id`,
and start later scripts with `await neo.task(id)`. The latest task pick wins: built-in
`page` uses its last-used open tab, else another open tab, else a new one;
`context.newPage()` opens in the picked task, and `task.newPage()` opens in that task.
Popups join their opener's task. `task.pages()` lists its tabs oldest first;
`task.page(tabId)` picks one. Finish with `await task.close()` to close its tabs and end it.

`context.pages()` lists the tabs a run may use, with `tabId` and `taskId` (`null` without a
task): every tab for a run without a task, and only its task's tabs for a run that carries one
under `tabs.scope: "task"`, the default ([task scope](neo-server/README.md#task-scope)). Recover a lost id with
`context.pages().find(p => p.url().includes('ycombinator'))?.taskId`.
An unknown id throws `task_not_found`, whose hint points to `context.pages()` and `page.taskId`.
The reply is exactly what the script returned, plus console lines and existing notes.
Hosts get touched tasks in `_meta["com.browseros.neo/run"].tasks`; host task ids and the
neo-viewer proxy keep their existing behavior. Tasks live in memory (at most 4096). A restart
of Neo's server gives each task back the tabs it held, as long as the browser kept them; a browser
restart forgets every task. The janitor closes agent-opened tabs nobody used for the host's `tabs.idleCloseSeconds`, 30 minutes by default.

```text
host / MCP run
      |
   neo-server -----> neo-playwright <--- neo-playwright-js (builds its assets)
      |                    |
      +----> neo-http -----+----> neo-trace
      |          |                    ^
      |          +----> neo-takeover --+
      |          +----> neo-sdk-api (spec + generated types)
      |          +----> neo-screencast <----- claw-server (desktop)
      +---------------> neo-captcha <----- claw-server (desktop)
```

| Package | Responsibility |
|---|---|
| `neo-server` | Stateless MCP `run`, JavaScript runtime, dispatch and application wiring. Builds `browseros-claw-sdk`. |
| `neo-playwright` | Browser operations, locators, snapshots and the isolated-world engine boundary. |
| `neo-playwright-js` | JavaScript probe and agent prelude; builds the engine's committed assets and fixtures. |
| `neo-http` | Authenticated host API, traces, live views, takeover routes and the edge of the host's private typing. |
| `neo-takeover` | Tab locks, display control, viewport fitting and owner handoff. |
| `neo-trace` | Run and operation records, storage and bounded trace values. |
| `neo-sdk-api` | Host API specification, generated Rust models and compatibility rules. |
| `neo-captcha` | Shared CAPTCHA solver, also used by desktop. `testbed/` checks it end to end through `run` ([README](neo-captcha/testbed/README.md)). |
| `neo-screencast` | Shared capture hub, also used by desktop. |

A host types a secret into a page with `POST /v1/tabs/{tabId}/fill`, never in a script: the
route lives in `neo-http`, and `neo-server` runs it as a run of its own with one `protected`
operation that records no value, address or picture (`neo-server/src/host/fill_service.rs`;
[the image's host typing guide](neo-server/docker/README.md#type-a-hosts-secret) describes its use). Neo also stores no picture of a
tab while any frame of it shows a box masking treats as private, not empty and painted in clear,
such as a card number a script typed; a password box paints bullets, so sign-in pages keep theirs.
Otherwise a tab that took a host's fill is pictured like any other by default; a host that wants no
picture of it sets `fill.pictures` to `never` ([the server's settings](neo-server/README.md#settings)).

`neo-server/docker/` builds the Neo SDK image, the browser plus this server.
See [the image's README](neo-server/docker/README.md) for host integration.

The pre-existing browser infrastructure (`browseros-cdp`, `browseros-core` and the other
workspace crates) stays outside this folder. Desktop depends on `neo-captcha` and
`neo-screencast`; the SDK server and its other libraries do not enter desktop's dependency graph.

## Phone takeover resolution

A host can request a sharper phone view through the authenticated host API:

```json
{
  "tabId": 123,
  "viewer": { "width": 366, "height": 700, "scale": 2 }
}
```

`POST /v1/takeovers` and `POST /v1/tabs/{tabId}/fit` accept logical viewer dimensions.
An omitted `scale` starts a new fit at 1. The accepted values are the integers 1 and 2.
Scale 2 requires a logical width of at most 600 pixels. Each logical side must be between
100 and 4096 pixels, and each side multiplied by scale must stay at or below 4096 pixels.
Invalid requests return HTTP 400 with `invalid_request` before changing the display.

At scale 2, a 366 by 700 phone uses a 732 by 1400 framebuffer. The page keeps its existing
mobile device size, touch behavior, and device pixel ratio of 2. The fit uses the browser's
existing 200 percent mobile preview zoom. It does not change the page's pixel density.
The physical display and browser window are shared, so another tab without emulation can
observe the larger display.

`Fit.screen` and `Fit.keyboard` use physical framebuffer pixels. Scaled fits also return
`Fit.viewer` with the logical size and active scale. A host maps its displayed canvas to the
framebuffer before sending RFB pointer coordinates. Agent locator boxes and mouse positions
remain logical CSS coordinates. Screenshot density follows the page's DPR and the screenshot's
own `scale` option, independently of the viewer scale.

Refit an active tab through `POST /v1/tabs/{tabId}/fit`. Omitting `scale` preserves its active
scale, and an explicit 1 returns it to 1x. Repeating takeover start returns the existing
takeover without refitting it. All active fits follow the latest shared display layout.
A host that sends remote framebuffer resizes must send physical dimensions at the active
scale, or fit its local canvas without resizing the remote display.
At 2x, an odd physical side rounds down to whole logical pixels and can leave one unused
edge pixel. An external resize past the logical phone width limit returns all fits to
desktop at 1x. A later resize does not restore 2x without an explicit refit.

Release restores the tab's original viewport request, including its previous preview zoom.
The last fit also restores the resting display size and saved window state. These use the
existing best-effort restoration path. A failed or timed-out release can leave restoration
incomplete, and restoring geometry cannot undo page events or requests caused during the fit.

Original viewport requests and window states are held in memory. A server restart forgets
active fits. Startup attempts to return the display to its configured home size and fullscreen
windows to maximized, but it cannot recover the agent's original custom viewport or window
state. This existing recovery limit also applies to unscaled takeovers.
The default takeover blocks agent reads, including screenshots. Use `reads: "allow"` when
scripts need to capture the tab during a takeover.

### The hand-back

The owner may open tabs during a takeover: a sign-in popup of the shared tab, or their bank by
hand. Neo gives none of them to a task. The release, whatever ended it (done, idle, the host,
the TTL, the tab closing), lists them in `openedTabs`: every tab that first appeared while the
takeover was active and was still open when the release began, each with its `url`, `inFront`
(the tab the owner left in front), `taskId` (the task holding it, or `null`) and `openerTabId`
(the tab that opened it, or `null` for one opened by hand). `GET /v1/takeovers/{id}` answers the
same list once released, and a repeated release answers the first one's. Each tab stays with
the task holding it, or with no task (hidden from bound runs, never closed by the janitor); the
host acts on the list by its own rule. Takeovers active at once each list a tab opened during
both. If the release cannot list the browser's tabs within 2 s, it still lists every tab it
kept, with `url` and `inFront` `null` (unknown), so a host never misses one the owner opened.
A takeover keeps the newest 256 tabs opened during it and counts them all: the release says how
many in `openedTabsNoted`, and `openedTabsTruncated` is `true` when more opened than it kept.
Then the list is not every tab the owner opened, so treat any tab it does not list as unknown
(for example, mark every tab no task holds): unknown never reads as absent.

From `packages/browseros-agent`, run the existing SDK gate with private build state:

```sh
CARGO_BUILD_BUILD_DIR="$PWD/target/build-private" crates/neo-sdk/check.sh
```

The gate covers every Rust crate in this folder: rustfmt, clippy `-D warnings` and tests for each
(not yet `neo-captcha`'s tests, which are flaky; `check.sh` says why), plus Google's rustfmt
options and the forbidden-dependency check for `neo-server` and `neo-playwright`. Follow [STYLE.md](STYLE.md) and the workspace's
contributor rules. Build the server with `cargo build -p neo-server --bin browseros-claw-sdk`.
Use Bun 1.4.2 in `neo-playwright-js` for `bun run build`, `bun run build:check` and its tests.
Regenerate the host contract from the agent root with `bun run codegen:neo-sdk-api`; never
edit `neo-sdk-api/src/generated` by hand.

Release and protocol identities remain `claw-sdk`, `browseros-claw-sdk`, the existing version
marker, MCP name and `CLAW_*` environment variables. The package rename changes tracing
targets to `neo_captcha::…` and `neo_screencast::…`, including in desktop logs. Custom `CLAW_LOG`
directives naming either crate must use its new target; the default filter is unchanged.

The server crate now uses `neo_server::…` tracing targets. Custom `CLAW_LOG` directives
for its former crate target should use `neo_server`; the variable and default filter stay the same.
