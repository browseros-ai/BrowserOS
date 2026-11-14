# claw-sdk

claw-sdk is hosted BrowserOS Neo's sidecar. Chromium launches it beside the browser, and agents talk to it instead of the desktop claw-server. It serves one stateless MCP tool, `run`, which executes an agent's JavaScript against the browser. Every SDK operation that script performs goes through one dispatcher, which owns the rules, delivery, and the trace.

The binary is `browseros-claw-sdk`. Code style and review rules are in [STYLE.md](STYLE.md).

## Hosting the real SDK in Node

The default-off Cargo feature `hosted-run` exports `hosted::{HostedRuntime, HostedOptions,
HostedRun}`. It builds the real trace store, browser link, Playwright
facade, built-ins, dispatcher and tab janitor. It binds no listeners and disables the trace
screenshot queue. Feature-off builds retain the product path.

Each connection owns an `App::runtime()` executor with the product's worker stack size.
`open_run(timeout)` starts a real `RunCtx` and seeds the same tab listing as MCP runs.
`invoke(name, json, site)` enters that executor and calls `Bridge::submit` synchronously; its
returned future always resolves to the bridge envelope. `log` and `sleep` use the same bridge.
`finish()` stops, drains operations, builds the reply and records the end exactly once; a
deadline or dropped run also ends through that path. `close()` drains runs, flushes traces
and releases the private executor, including core's background tasks and CDP socket.

`state_dir` is required; the store is `<state_dir>/claw-sdk/trace.db`. Pass the installed
app's version as `browseros_version` for native capability gates. `execution_dir` is accepted
as a legacy launch fact; it imposes no upload path policy. Set `hosted_mode` when the browser
was launched with `BROWSERCLAW_HOSTED_MODE=1`; it is off by default, as the spec runner launches
BrowserOS unless given `--hosted-mode`. An initial run has `init.page = null`; `init.pages` includes
all existing tabs, including the launch tab. The lazy default page opens when the SDK needs it.

The isolated N-API crate is `../neo-playwright-tests/node/claw-addon` (its own workspace and
lockfile, seeded from the agent lock). From `packages/browseros-agent`:

```sh
cargo build --release --locked --manifest-path crates/neo-sdk/neo-playwright-tests/node/claw-addon/Cargo.toml
cp "$CARGO_TARGET_DIR/release/libclaw_node.dylib" crates/neo-sdk/neo-playwright-tests/node/claw-addon/claw_node.node
node --test crates/neo-sdk/neo-playwright-tests/node/claw-smoke.test.mjs
```

The loader `openClaw(cdpPort, { timeoutMs, stateDir, executionDir, browserosVersion },
{ onUncaught })` returns the real SDK parameters, the native run, runtime, source hashes and
an idempotent `finish()`. `CLAW_NODE_ADDON` overrides the add-on path. Omitted `stateDir`
uses a temporary directory removed at close. The default run timeout is 24 hours.

The loader reads bootstrap, Playwright prelude and Neo prelude verbatim, adds the production
strict-mode prefix and evaluates them with `vm.compileFunction` in Node's main realm. A
`globalThis` parameter holds the natives and `__neo` privately. The SDK's Promise, Error and
Uint8Array have Node's identity; QuickJS's memory and CPU limits do not apply. `__neo.run`
keeps every provided parameter and stays pending until finish. Listener throws surface through
Node's `unhandledRejection` event and are forwarded to `onUncaught` for the worker's lifetime;
existing Node listeners still observe them. Use one loader run per worker.

Web globals (`URL`, `URLSearchParams`, `atob`, `btoa`, `DOMException`) also come from Node's
main realm, as in the prelude's VM test rig. The loader does not evaluate `web_globals.js`:
its factory requires QuickJS's Rust natives. Production URL parsing uses Rust's `url` 2.5.8,
which retains older opaque-path trailing-space rules and exposes no `URL.createObjectURL`
or `URL.revokeObjectURL`. Node uses its own URL implementation and blob URL methods, so the
comparison does not prove parity for these globals. `sources` still hashes the three
production bootstrap and prelude files.

The smoke skips without `NEO_E2E_BROWSER`. It owns a headless browser on a throwaway profile,
verifies its process group before cleanup, and requires its Node child to exit naturally.
The primary launch leaves `BROWSERCLAW_HOSTED_MODE` unset to match the comparison runner;
`CLAW_SMOKE_HOSTED=1` repeats it with BrowserOS hosted mode and reports console, popup and
`setContent` differences. `NEO_NODE_OUTPUT_DIR` retains proof, logs and cleanup records.

## How it runs

To run it beside the browser in a container, use the Neo SDK image in [docker/](docker/README.md).

Chromium starts the binary with its launch JSON and reads the exit code.

| Invocation or event | Result |
|---|---|
| `--version` | Prints the version alone on stdout and exits 0. Reads no config and opens no state. |
| `--config <launch.json>` | Serves until SIGTERM or SIGINT, then exits 0. |
| The agent port is taken | Exits 2, and Chromium relaunches Neo on the next port. Nothing else exits 2. |
| Any other failure | Exits 1, bad arguments included. |

From the launch JSON it reads `ports.server` (default 9200), `ports.cdp` (default 49337), `directories.resources`, `directories.execution` (Chromium's `<user-data-dir>/.browseros`), and `instance.browseros_version`. It ignores every other key. claw-sdk has no config file of its own, so it logs and ignores `BROWSERCLAW_CONFIG_FILE`.

`src/config/env_config.rs` reads the environment once, at start. A blank value counts as unset. A malformed address or screen size fails the start with exit 1.

| Variable | Default | Effect |
|---|---|---|
| `CLAW_DOWNLOADS_DIR` | `$HOME/Downloads` | The browser downloads folder and default `files.readDirs` entry. With no override and no discoverable home, the default is an empty list and startup logs a warning. |
| `CLAW_PROFILE_DIR` | launch profile | Protected browser profile. Without an override, `directories.execution` named `.browseros` identifies its parent as the profile; otherwise startup warns that the profile is unprotected. |
| `BROWSERCLAW_DIR` | `$HOME/.browserclaw` | The state directory. The start fails when both it and `HOME` are unset. |
| `CLAW_LOG` | `info,rmcp=warn` | The `tracing` filter. A filter that does not parse falls back to the default with a warning. `rmcp` is capped at info, and `sea_orm` and `tungstenite` at debug, in both log sinks; explicit crate directives cannot enable MCP request/reply or WebSocket frame dumps. |
| `BROWSERCLAW_HOSTED_MODE` | unset | Must be `1`. Any other value logs an error, because popups then open as windows Neo cannot drive. Neo still starts. |
| `CLAW_NATIVE_BROWSEROS_VERSION` | the launch JSON's | Overrides the browser version that gates core's native features. |
| `CLAW_HOST_SOCKET` | `$BROWSERCLAW_DIR/host.sock` | The host API's Unix socket. |
| `CLAW_HOST_TOKEN_FILE` | unset | The host's bearer token. Without a usable token (32 or more bytes of visible ASCII) there is no host listener, and `run` still works. |
| `CLAW_HOST_TCP` | unset | Also serves the host API on this TCP address. |
| `CLAW_XVNC_ADDR` | `127.0.0.1:5900` | Xvnc, which a takeover fit resizes. |
| `NEO_SCREEN` | `1600x1000` | Neo's screen size between fits. |
| `BROWSEROS_CAPSOLVER_API_KEY_FILE`, `BROWSEROS_2CAPTCHA_API_KEY_FILE` | unset | CAPTCHA provider keys. Without a readable one, `neo.solveCaptcha` is unlisted and fails `host / captcha_unavailable`. neo-captcha also reads `BROWSEROS_CAPSOLVER_API_URL`, `BROWSEROS_2CAPTCHA_API_URL`, and `BROWSEROS_CAPTCHA_PAID_TASK_LIMIT` (default 100). |

Neo binds two listeners:

- The agent listener is `127.0.0.1:<ports.server>`, with no auth. Chromium's proxy forwards to it. It serves `/mcp`, plus `GET /system/health` and `GET /system/ready` for Chromium's supervisor. `/mcp` refuses any request that carries `Origin` or `Sec-Fetch-Site`, so a web page in the driven browser cannot call `run`.
- The host listener is a Unix socket with mode 0660, plus TCP when `CLAW_HOST_TCP` is set. It serves neo-http's `/v1` routes (runs, operations, screenshots, events, locks, tabs, fits, and settings) behind the service's bearer and peer-uid checks. The service starts when the browser link first connects.

State lives under `$BROWSERCLAW_DIR`:

- `claw-sdk/trace.db` is the SQLite trace store. Supported older files are upgraded in place. If it does not open (including an unknown newer migration or the earliest session-era schema), Neo moves it aside to `trace.db.broken-<millis>` and starts a fresh one. Rolling back across a newer migration loses the live trace and request-id history: the older image still starts, logs `the trace store does not open; starting a fresh one`, lists none of the runs from before the rollback, and runs a `requestId` sent before it again instead of replaying it. Only `trace.db` moves (its `-wal` and `-shm` files are removed, with any writes not yet copied into it); `settings.json` stays. Moving forward again upgrades the older image's store, and the earlier runs stay only in the `.broken` file, which Neo never reads or deletes. Schema 5 (`m0005`, the operations' `framesIncomplete`) was proven this way on a schema-4 image.
- `logs/claw-sdk.log.<date>` is the daily log. The same lines go to stderr. `logs/claw-sdk-panics.log` gets each panic, written synchronously.
- `claw-sdk/settings.json` is a host's startup settings or changes through `/v1/settings` (below), mode 0600.
- `host.sock` is the host socket, unless `CLAW_HOST_SOCKET` moves it.

### Settings

A host reads settings through `GET /v1/settings` and changes them through `PUT` or `PATCH`, described in [openapi.yaml](../neo-sdk-api/openapi.yaml), or writes `$BROWSERCLAW_DIR/claw-sdk/settings.json` before startup. The file holds one `PUT /v1/settings` body with no envelope, such as `{"tabs":{"idleCloseSeconds":300},"trace":{"values":"off","screenshots":"off","retentionSeconds":3600}}`. Every setting is optional. A script cannot write this file or reach the host listener.

| Setting | Default | Range | What it changes |
|---|---|---|---|
| `tabs.idleCloseSeconds` | 1800 | 60 to 86400 | The janitor closes the agent's tabs, and popups they opened, once nobody used them for this long. A tab is in use while a run that used it is in flight, a lock holds it, or a fit is on it; its clock restarts when the last of those ends. Neo's own blank tab keeps the browser open, so every idle agent tab may close. The check runs once a minute. |
| `tabs.scope` | `task` | `task`, `browser` | Which tabs a run sees (see [Task scope](#task-scope)). `task`: a run that carries a `taskId` sees and acts on only its task's tabs; a run without one sees every tab but cannot take over a task a host named. `browser`: every run sees every tab, whatever task it carries. A run reads it at its start and keeps it to its end. |
| `trace.retentionSeconds` | 604800 | 3600 to 2592000 | Runs older than this are deleted with their operations, screenshots and Neo's copy of their downloads, and a repeat of their `requestId` runs again. The sweep runs every 10 minutes. A file the browser saved stays. |
| `trace.screenshots` | `perOperation` | `perOperation`, `off` | Neo's own screenshots: after each write, after a failure the website or the script caused, and of each tab a run touched when it ends. `off` takes none from the next operation that ends; a script's own `page.screenshot()` is its result, whose recording follows `trace.values`. |
| `trace.valueBytes` | 0 | 0 to 262144 | Keeps each operation's arguments and result, a run's result, and any one string in them whole up to this many bytes. `0` keeps the trace's own limits: 4 KiB arguments, 2 KiB results, 1 KiB strings. The Neo viewer sets 262144, Neo's reply limit. It replaced the environment value `CLAW_TRACE_VALUE_BYTES`, which Neo now only warns about. |
| `trace.values` | `on` | `on`, `off` | `off` omits script text, arguments, results, notes, host data, stored replies and error messages from records at their end. Metadata and downloads stay. Blob results written with `off` expire ten minutes after their write; the next minute upkeep removes them. Existing records stay. |
| `files.readDirs` | downloads folder | 0 to 16 folders | Absolute folders scripts may read files from by path. Resolved paths must lie inside a listed folder. Changes apply at the next file operation, including a run already in flight. |
| `fill.pictures` | `allowed` | `allowed`, `never` | Whether a tab that took the host's private fill is pictured. `allowed`: like any other tab, by a script's `page.screenshot()`, `locator.screenshot()` and `page.pdf()` and by Neo's own screenshots. `never`: no picture of it for the tab's life (`private_tab` for a script), kept across a restart of Neo by Neo's record of filled tabs, which a fill writes before its first character only under `never`. A picture reads it when it is asked for; a fill reads it when it starts. Under either value Neo's own picture is withheld while a frame shows a private box's value in clear. |

With `trace.values: "off"`, records carry `values: "off"` and errors retain their owner and
code with an empty message. The host's live reply is unchanged. A repeated request gets its
reply from memory for ten minutes; after a server restart it gets `duplicate_request` and the
run's id. Neo still lists downloads and serves their copies until retention. A host that also
wants no automatic pictures sets `trace.screenshots: "off"`.

`trace.values` governs Neo's own records and log lines; other libraries can print values when a
host raises `CLAW_LOG`, so a host that needs off keeps the default `info,rmcp=warn` filter.

Every setting is optional. Trace defaults and `tabs.idleCloseSeconds` preserve the existing behavior; `tabs.scope` defaults to `task`, which binds only runs that carry a `taskId` ([Task scope](#task-scope)); `files.readDirs` defaults to the browser downloads folder. `fill.pictures` defaults to `allowed`, a decision of 2026-11-13, where before a filled tab was never pictured.

What a model can see by default: a script can take a picture of a tab after a private fill. A password box paints bullets. A card number, a security code and a one-time code are painted in clear, so a model that takes a screenshot can see one that Neo typed privately. It still cannot read it as text. A host that does not want that sets `fill.pictures` to `never` before the first fill. `never` covers every tab Neo knows took a fill: those filled in this run of the server, whatever the setting was, and after a restart only those filled while it was `never`. `PUT` sets every setting, and one left out goes back to its default; `PATCH` changes only the settings it names. A name Neo does not know is skipped, never refused, and the answer lists it in `unknown`, so a host and a server of different versions still work together. A name of an open list Neo does not have yet, such as a later `trace.screenshots` mode, is skipped the same way, and the answer names its setting in `unknownValues`, never the value. A known setting with a value outside its rule refuses the whole call with 400 `invalid_request`, which names every setting at fault and its rule; nothing is applied. A change takes effect at once, with no restart: each part reads its setting when it acts. `tabs.scope` is read once, at a run's start: a run in flight keeps the scope it started with, and the next run takes the new one.

Write the whole file under another name in the same folder and rename, or write while Neo is stopped. After startup Neo owns the file. Change a running server through the API, which saves before switching values on. Neo loads the saved file at every server start, before the first run, with or without a usable `CLAW_HOST_TOKEN_FILE`. A saved `fill.pictures` that this server cannot use falls back to `allowed`, so a host that needs `never` reads `GET /v1/settings` after start.

A missing file means normal defaults. An unreadable file or input that is not a JSON object uses defaults except that `trace.values` and `trace.screenshots` are off. An invalid known value for either switch uses off, and an invalid `trace` group turns both off. Other invalid values keep their normal defaults, including retention, so a bad file never shortens existing history. Unknown names are skipped. Neo logs each case without quoting invalid values and always starts. Recording-off fallbacks appear in `GET` and stay in the next API save.

In the [SDK image](docker/README.md#use-it-from-a-host), the saved path is `/data/browserclaw/claw-sdk/settings.json`. Its entrypoint takes the state folder and file for Neo's configured user, with mode 0600 for the file. A regular `/etc/neo/settings.json`, mounted read-only or copied into a derived image, atomically replaces the saved file at every entrypoint start. API changes survive server and browser restarts; the next entrypoint start restores the fixed file's settings. A failed copy is logged and leaves the saved file in use. Without a fixed file, saved changes persist across container starts.

Adding a setting is one row in `neo-http/src/setting.rs`, its key in the `Settings` schema of `openapi.yaml`, a row in the table above, and a reader where Neo uses it. A test in `setting.rs` keeps the three lists equal.

Path reads in `setInputFiles`, `fileChooser.setFiles`, `drop`, script/style tags and page/context init scripts follow `files.readDirs`. The default allows uploading a file returned by `download.path()` and a copy saved inside the downloads folder. Owner files can arrive as `{ name, mimeType, buffer }` payloads without a host staging folder. A host that stages elsewhere adds that absolute folder with `PATCH /v1/settings`, for example `{"files":{"readDirs":["/data/downloads","/run/uploads"]}}`.

The check resolves symlinks and compares path components at each use. Missing roots match nothing; `[]` denies all path reads. A relative path uses Neo's working directory. Allowed aliases keep their supplied filenames. Regardless of the list, scripts cannot read or overwrite Neo's state folder, browser profile, host token or either configured CAPTCHA key file. In hosted mode (`BROWSERCLAW_HOSTED_MODE=1`), the server's home is protected the same way: `HOME`, else the passwd entry's home, where the browser and its helpers read files without being asked. The exception is the downloads folder, or a `files.readDirs` folder, that lies in the home and whose first name below it is not a dot-entry. A save into such a folder may make it and the missing folders above it inside the home, and no other folder there. The rule judges real paths, and with no home found, startup logs that the home is not protected. The Node embedding never protects its host's home. A directory upload containing one of those locations is refused, including `/` with `["/"]`. Download and screenshot saves elsewhere keep their existing behavior. A refused save creates no parent folders and reports `script / invalid_input / not_sent` without exposing the path.

The Node embedding accepts `HostedOptions.read_dirs`; omission uses the same downloads default and `Some(vec![])` denies path reads. In-memory payloads and clearing an input with `[]` work with every list. Payloads total under 10 MiB a call, and a directory upload needs its tab in front.

## The `run` contract

The realm provides `URL`, `URLSearchParams`, `atob`, `btoa` and `DOMException`. Known URL differences: `url` 2.5.8 retains the older opaque-path trailing-space rules; `URL.createObjectURL` and `URL.revokeObjectURL` are absent.

The model sends `run { code, timeout? }`. `code` is the body of an async function whose parameters are the SDK: Playwright's `page`, `context`, `browser`, `expect` and `playwright` (its `selectors` and `errors`), and Node's `Buffer` (neo-playwright's prelude), `neo`, `console`, the timers, and the error classes. `timeout` is in milliseconds, from 1 to 900000, and defaults to 300000. Unknown fields are refused. Each run gets a fresh QuickJS realm. `src/mcp/run_description.rs` builds the text the model reads about all of this.

The host sends its ids in `_meta["com.browseros.neo/run"]` as `{ requestId?, taskId?, hostData? }`. Neo refuses unknown keys as `host / invalid_meta`. `hostData` is up to 2048 bytes of JSON that Neo stores with the run and never reads.

A host's `taskId` is also a Neo task id and seeds the run's current task. Neo registers it when a run first opens a tab for it; `neo.task(hostId)` can then fetch it. After a restart of Neo's server it starts on the same tab as before (see below). The neo-viewer proxy still supplies the host's task metadata.

With no task selected by the host or script, built-in `page` opens a new blank background tab on first use, every run. `context.newPage()` also opens a new tab. Neither reuses an existing tab or creates a task.

To keep tabs across runs, call `const task = await neo.createTask(name)` and return `task.id`. Start later runs with `await neo.task(id)`. The latest `neo.createTask` or `neo.task` call picks the script's current task:

- Built-in `page` uses that task's last-used open tab, else another open tab in it, else a new tab in it. `neo.task(id)` binds it to that open tab at once, so `page.url()`, `page.tabId` and `page.taskId` are the tab's on the next line (a tab whose host lock blocks reads binds with nothing to read). If the run already holds a Page for that tab (from `context.pages()`, a result or an event), that Page stays the tab's, with its listeners and bindings, and `page` takes the tab at its first action instead. Without an open tab to bind, it picks the tab at its first action; until then `page.tabId` and `page.taskId` are null and `page.url()` is `about:blank`. Once `page` has a tab it keeps it for the run: later picks steer new tabs only.
- `context.newPage()` opens in the current task; `task.newPage()` opens in that task. Popups join their opener's task.
- `task.pages()` lists its tabs oldest first; `task.page(tabId)` picks one. Scripts may declare their own `const page`.
- `await task.close()` closes the task's tabs and ends it.

`context.pages()` lists each page the run may use (every tab, or with a task under [task scope](#task-scope), that task's tabs) with `tabId` and `taskId` (`null` without a task). Recover a lost id with `context.pages().find(p => p.url().includes('ycombinator'))?.taskId`. An unknown id throws `task_not_found`, whose hint points to `context.pages()` and `page.taskId`. See [agent.d.ts](../agent.d.ts) for the declarations and the four-run walkthrough.

`page.frames()` of every page a script reaches (its built-in `page`, each of `context.pages()`) is the tab's whole frame tree, cross-site frames included, from the script's first line, and a tree that changed is read again before the script's next operation settles. A synchronous `page.on('popup')` or `context.on('page')` callback may get the new page before its frames are read, while the operation or `waitForEvent` that delivers it resolves on its whole tree, as Playwright's `popup` event also fires before the popup's children load. A tree still not whole after one retry lists the frames that were read, the server logs a warn line naming the tab and the error, and the tree is read again before the next operation settles; `frames()` does not throw. Each of these reads is an operation in the run's trace (`internal: true`, so `internal=false` leaves it out). A host finds a short read as `framesIncomplete: true` on the host API's operation routes, such as `GET /v1/runs/{runId}` (full or `fields=summary`). The mark describes that one read: a later read of the same tab may be whole, and `false` on a read that failed (refused, timed out) says nothing about the tree.

Tasks live in memory, at most 4096. Empty tasks remain until closed or evicted for space. Which task holds which tab is also saved, by the browser's target id, in `tab-holders` beside the trace (one JSON line per change, unsynced). When Neo's server restarts and the browser keeps its tabs, each task gets back the tabs it held (the ones it opened, their popups, the tabs of tasks its runs created) and its last-used tab, before the first run starts; the wait is bounded by that listing of the browser's tabs (three tries of 3 seconds, so about 10 seconds at most). A task with no open tab is not saved. A browser restart gives nothing back: tab numbers can repeat in a new browser, target ids never do, so its tabs hold no task. If the file cannot be read, or the browser's tabs cannot be listed at start, no task gets tabs back and the file is left as it was. `GET /system/health` says how this start's restore ended, in `tabHolders.state`: `pending` until it ends, `restored` once it ran (also when no saved tab was still open, as after a browser restart), `unreadable` when the file could not be read (nothing restored, and nothing saved until the next start), or `unlisted` when the browser's tabs could not be listed (nothing restored). It names no tab and no task. Under task scope a lost record shows only as tasks losing their tabs after a restart of Neo's server, so this tells a host that was Neo's file. The Node embedding keeps no record and has no such field. Tabs from before a restart count as the owner's, which the janitor never closes. It closes the agent's tabs once nobody used them for `tabs.idleCloseSeconds` (30 minutes by default; see Settings).

Closing the browser's last tab quits the browser, and Neo's server with it, so Neo keeps one blank tab of its own open at all times (`browser/neo_tab.rs`). At the browser link's first connect and after each reconnect it adopts a blank tab no task owns (the launch tab, or its own tab the browser restored), else it opens one in the background; if that tab closes while Neo runs, it opens another, and every close Neo makes (a script's `page.close()`, with or without `runBeforeUnload`, `task.close()`, the janitor) first makes sure it is open. If Neo cannot open it and the browser lists no other tab, the close fails `neo / cdp_error` with nothing sent, rather than quit the browser. So the janitor may close every idle agent tab, a script may close its last tab, and a host's tab reset needs no keeper tab of its own. No script sees or reaches it: `context.pages()`, task listings and page events leave it out, and an op that names it fails `script / invalid_input` with nothing sent. Apart from task scope, this is the one exception to `context.pages()` listing every tab. Neo keeps the tab only while it is blank: once the owner browses in it (in a fresh browser it is often the tab they see) or a fit's navigate moves it, Neo opens a new blank tab and then hands the old one back as an ordinary owner's tab. A run's initial pages and `context.pages()` check for this first, so a script lists the owner's page at once; otherwise Neo notices within 30 seconds. A context's `page` listeners hear of the handed-back tab as of a tab the owner opened, and the janitor never closes it. The host sees it in `GET /v1/tabs` as `owner: "neo"`. The janitor's own check that never closes the browser's last tab stays as a fallback. The Node-hosted embedding keeps no blank tab: its host launched its own browser.

The result carries `_meta["com.browseros.neo/run"]` as `{ id, status, effects, error?, unknown, tasks, tabsScope }`. `id` is the run's id in the trace. `tabsScope` is `task` when the run was bound to its task's tabs and `browser` when it saw every tab; a refused run says what it would have been, and a repeated request's reply carries the first run's. An older Neo sends none, which means `browser`. `status` is `ok`, `error`, `cancelled`, or `timeout`, and `isError` is set unless it is `ok`. `effects` is `none`, `done`, or `unknown`. `unknown` lists the writes that were sent and never confirmed. `structuredContent` is unchanged.

`tasks` is host metadata, always present (`[]` when no tasks were touched). Each entry carries `id`, `name`, `kind`, and `tabs: { tabId, url }[]`. Tasks appear once, in first-touch order, even when the script failed. Tabs and URLs reflect reply time; a closed task has kind `closed` and no tabs.

The agent's reply is exactly what the script returned, plus console lines and existing notes. Task metadata adds no reply text. Return `task.id` explicitly to keep it for later runs.

The script's `page`, `context`, `browser` and `expect` behave as Playwright 1.64.1's, except where Neo says otherwise in the `run` description, which lists each difference a script would trip on. A call that waits gives up after 5 s unless given a `timeout`, and a navigation after 30 s. For the main-world scripts (`page.evaluate`, `page.waitForFunction`, `locator.evaluate`, `locator.evaluateAll`, `locator.waitForFunction`, `elementHandle.evaluate`, `$eval` and `$$eval`, each `frame` form in its own frame) and `setContent`, whose HTML's scripts run in the main world too, those are:

- `waitForFunction` polls every 100 ms for its default `polling: 'raf'`, because animation frames never fire in Neo's background tabs. The handle it resolves to holds a snapshot of the value, so `jsonValue()` reads it and `evaluate` on it fails `neo / unsupported`.
- A result nested more than about 50 objects deep fails its call with `Maximum call stack size exceeded` (`page.evaluate: Maximum call stack size exceeded`, an `Error` with no owner or code, since the op answered), where Playwright allows 98. The prelude's result parser recurses on the realm's 512 KiB stack (`Defaults::JS_STACK_BYTES`); 50 was measured in a debug build.
- `setContent` refuses `waitUntil: 'networkidle'` as `neo / unsupported`, with nothing sent. Chromium never fires `networkIdle` for a written document, and Neo does not count the page's requests itself, so the wait could only time out. Use `load`.
- An owner's lock that blocks reads, taken while a script runs, withholds what the script read: the call fails `host / owner_has_control`, delivery `done`.

`page.addInitScript` and `context.addInitScript` register a script that every new document runs in its main world, before the page's own scripts, as Playwright's do. They differ in these ways:

- A script lasts as long as the run that registered it. Neo's tabs outlive a run, so when the run ends, however it ends, Neo removes the script and the tab's later documents no longer run it. Within the run it survives every navigation, and the returned disposable removes it sooner.
- Neo's context is the owner's whole profile, so a context's script does not go to every tab at once. It reaches each tab at the run's first write to that tab, after the door's rules on the tab, before the write: every document the run loads runs it, and a popup's first document, which loads before the run acts on the popup, does not.
- Neo adds a script to the tab's page session, which covers the main frame and its same-process frames. A cross-site iframe has a session of its own and runs without it.
- `exposeFunctions` passes the script's functions to the page as bindings, below.

`page.exposeBinding`, `page.exposeFunction`, their `context` forms and the `exposeFunctions` option of `addInitScript` and of `evaluate` (page, frame, locator and element handle) let the page call a function of the script and get its result back, as Playwright's do. The page's call goes to CDP's `__playwright__binding__` (in the main world only), Neo records it, the prelude's event pump runs the function in the run's realm, and Neo answers in the execution context that called. They differ in these ways:

- A binding lasts as long as the run that exposed it, since the realm that answers it ends with the run. When the run ends Neo removes it from the tab's later documents and from the ones loaded then, so the page can no longer call into a run that is gone. `dispose()` does the same sooner.
- A context's binding reaches each tab as a context's init script does, at the run's first write to it. A cross-site iframe has no bindings.
- A call reaches the script through the event pump, which is not ordered against op replies. A call the page awaits (`await page.evaluate(() => window.add(1, 2))`) answers before the op returns; one the page fires and does not await, such as from an event handler, can run after the op that caused it returned.
- A binding answers with values; a handle in its result fails the page's call. `evaluateHandle` has no `exposeFunctions`, since Neo returns no JS handles.
- A function `evaluate` passes stays exposed, off `window`, until the run ends. Playwright erases these when the main frame loads a new document; Neo's prelude does not see every new document, and a page there cannot reach them anyway.

A `requestId` never runs twice. A repeat while its run is going joins it and gets the same reply. A repeat within 10 minutes of the end gets the stored reply, from memory or from the trace store, so this holds across a restart. Later repeats are refused as `host / duplicate_request`, naming the first run. If the store cannot be read, Neo refuses the call as `neo / trace_unavailable` rather than risk a second run. Neo notices a gone client at the next SSE keep-alive, within 5 seconds. A run with a `requestId` then keeps going for 15 seconds, so a retry can join it. A run without one stops at once.

### Task scope

Under `tabs.scope: "task"`, the default, a run whose host sent a `taskId` sees and acts on only its task family's tabs. A family starts at the task a host named, and every task its runs create with `neo.createTask`, at any depth, joins it. The family's tabs are the ones its tasks' runs opened (built-in `page`, `context.newPage()`, `task.newPage()`) and their popups, so a run sees its own task's tabs, the tabs of the tasks it created, and its sibling tasks' tabs alike. A popup opened without an opener the page can reach (`window.open(url, '_blank', 'noopener')`, a `target=_blank rel=noopener` link) is its opener's task's too: Neo knows the opener even when the page does not. A task keeps its family for its life: a host that later sends the id of a task an agent created gets that task's family, not a new one. `GET /v1/tabs` names each tab's holder in `taskId`. `context.pages()` lists those tabs, and page events and `page.opener()` leave the others out. An operation that names any other tab, whether another task's, the owner's or a number that never was one, fails `script / page_not_in_task` with nothing sent, and its record names no tab. A tab of its own that closed fails `page_closed`. `neo.task(id)`, `task.newPage()` and `task.close()` answer a task outside its family as a missing one (`task_not_found`).

A run without a `taskId` is not bound. It sees every tab, as before, and may pick, open in and close tasks agents created, but never a task a host named. Under `tabs.scope: "browser"` no run is bound: every script may use any tab or task. Every result says which applied in `_meta["com.browseros.neo/run"].tabsScope`.

What a run without a `taskId` meets under the default, against a Neo before task scope. Its tabs are as before: it lists and acts on every tab but Neo's own blank one. The rest differs:

| What | Before | Now |
| --- | --- | --- |
| A task a host named: `neo.task(id)`, then opening a tab in it or closing it | Picked, opened in and closed | Answers `task_not_found`. Tasks agents created stay reachable. |
| A `requestId` that a run with a task already used | Repeated that run's reply, or was refused as a duplicate | Matches only earlier runs without a task, so the call runs again as a new run, with its effects. Under `tabs.scope: "browser"`, as before. |
| Each result's `_meta["com.browseros.neo/run"]` | | Gains `tabsScope: "browser"`. A reply stored by an older Neo reads `browser` too. |
| `GET /v1/runs`, a run's record, the trace tail | | Each new run gains `tabsScope`; older runs have none. |
| `GET /v1/tabs` | | Each tab a task holds gains `taskId`, its holder now. |
| `GET /v1/settings` | | `tabs.scope` in `settings` and `defaults`. A `PUT` that leaves it out sets it back to `task`. |
| `GET /system/health` | | Gains `tabHolders.state`, how the restore of which task holds which tab ended at start (see [The `run` contract](#the-run-contract)). |
| The run tool's description | | Says how a run's tabs are scoped. |
| The trace file, `claw-sdk/trace.db` | Schema 5 | Schema 6 (`m0006` adds `runs.tabs_scope`), upgraded in place at start. An older image cannot open it: it moves it aside and starts a fresh trace, without the runs and request ids from before (see the trace store above). |

So a host's own scripts sent without a `taskId` keep today's tabs, but must not use a task a host named, and a `requestId` first sent with a task does not repeat without one.

A host whose own scripts must reach every tab (resetting tabs, reading a kept tab, closing one after a fill) sends them without a `taskId`, or sets `tabs.scope` to `browser`. The scope is a wall in Neo's API, not web isolation: every task shares one browser profile, with its cookies, storage and downloads.

## Module map

| Module | Owns |
|---|---|
| `main.rs` | The process contract: flags and exit codes. |
| `app` | Startup in dependency order, shutdown on SIGTERM, and housekeeping (idle tabs and trace retention). |
| `config` | The launch JSON, the environment, and `Defaults`, which holds every fixed value. Nothing else reads an env var. |
| `logging` | The daily log file, stderr, and the panic log. |
| `http` | The two listeners, the request hygiene in front of `/mcp`, and `/system/*`. |
| `mcp` | The `run` tool: parsing its arguments and `_meta`, its description, and rendering a reply. |
| `run` | One `run` call from admission to reply: replay, joins, the orphan grace, and the deadline. |
| `runtime` | The per-run QuickJS realm, the JS bootstrap and the `neo` prelude in `runtime/js/` plus neo-playwright's prelude, and the strings-only bridge to the dispatcher. The only module that names rquickjs. |
| `dispatch` | The door: `Dispatcher`, the `Method` trait, the `Registry`, the policy rules, and the error model. |
| `builtins` | Registers neo-playwright's ops on the process's one `Playwright`, `neo.solveCaptcha`, and the idle-tab janitor. `op_marks.rs` maps capability declarations to dispatch metadata; main-world answers and pulled events pass the takeover rules on their targeted tab. File uploads use Playwright's path handling, and navigation delegates URL handling to Chromium. |
| `browser` | The CDP link to Chromium, its reconnects, and what Neo knows about each tab. |
| `host` | The host API's lifecycle and the takeover lock the door checks. |
| `test_rig` | The unit tests' rig: a fake browser, a real trace store, and test methods. |
| `test_support` | `FakeCdp` and a test config, behind the `test-support` feature that only test builds turn on. |

Every SDK operation goes through one door. A script's call reaches `Dispatcher::submit` through the bridge, and the idle-tab janitor submits its closes the same way. A method that needs another action calls `OpCtx::invoke`, which runs it through the same door as a child op. The door owns the op id, the trace records, the rules, and delivery, so a method does its work and reports facts. `neo.solveCaptcha` is the one method that drives a page outside the door, and it holds a claim on the tab while it does.

## Add a built-in method

A built-in is a private type in `src/builtins/` that implements `dispatch::Method`. Copy `src/builtins/solve_captcha.rs`. The Playwright API is not written this way: neo-playwright owns its ops, and `src/builtins/playwright_ops.rs` registers each one from the crate's table.

1. Write an input type with `#[serde(deny_unknown_fields)]` and an output type that implements `Serialize`.
2. Implement `Method`:
   - `spec` returns a `MethodSpec` with the name scripts call, its `Effect` (`Read` or `Write`), and its `Listing` in the `run` description.
   - `target` returns the input's `PageTarget`, or `None` for a method with no page. There is no default, so the takeover lock always sees the tab a method acts on.
   - `call` does the work through its `OpCtx<Self>`. Await browser calls through `op.race`. A write calls `op.recheck()?` and then `op.mark_sent()` right before it sends, and the door derives delivery from that.
3. Register it with `builder.define` from `builtin_set` in `src/builtins/builtin_set.rs`. A duplicate name fails the start.
4. Expose it to scripts from a prelude in `src/runtime/js/` with `__neo.invoke('<name>', input)`. A new parameter of the agent's function goes through `__neo.provide`, and a test in `run_description.rs` fails until the description names it.

## Test it

`check.sh` is the gate, and the package's `lint:rust` script. It runs rustfmt with Google's options, the forbidden-dependency check, clippy with `-D warnings`, and `cargo test`, all on CI's pinned toolchain. The same rustfmt (with the workspace's defaults), clippy and tests run on every other crate under `crates/neo-sdk` too, such as `neo-takeover` and `neo-trace`; `neo-captcha`'s tests stay out until two flaky ones are fixed (`check.sh` names them). Run it from `packages/browseros-agent`. In a Grove worktree, give it a private build directory, or it waits on the build lock that other worktrees hold:

```sh
CARGO_BUILD_BUILD_DIR="$(git rev-parse --show-toplevel)/packages/browseros-agent/target/build-private" \
  crates/neo-sdk/check.sh
```

Unit and integration tests run against `FakeCdp`. The `*_browseros.rs` integration tests drive a real headless BrowserOS instead. Stock Chrome does not work, because core uses BrowserOS-only CDP methods. The tests pass without doing anything unless `CLAW_SDK_E2E_BROWSER` names the browser binary:

```sh
CLAW_SDK_E2E_BROWSER=/Applications/BrowserOS.app/Contents/MacOS/BrowserOS \
  cargo test -p neo-server --locked --test e2e_browseros --test acceptance_browseros \
    --test playwright_browseros --test uploads_browseros --test served_methods_browseros \
    --test main_world_browseros
```

- `tests/e2e_browseros.rs` launches the built binary the way Chromium does, runs scripts over MCP, and reads them back over the host socket.
- `tests/playwright_browseros.rs` runs one Playwright script over MCP, the way a model trained on Playwright writes it. The script loads a page with a same-origin iframe into the run's own `page`, reads its `ariaSnapshot()` (the condensed v1 text, pinned line for line), reads elements by a printed ref through `getByRef` and `aria-ref=`, and reads through locators and inside the frame. It also takes `ariaSnapshot({ delta: true })` across runs of one task: the baseline is the last snapshot a run gave its caller, not one a failed run took, one the script only inspected, nor one taken earlier in the same run. It checks the reply's delta tip too: after a whole-page snapshot found a stored baseline, never on a failed run, three times per task and three per tab for runs with no task.
- `tests/uploads_browseros.rs` uploads through `setInputFiles` over MCP: host-owned files and their symlinks are refused; a file allowed by the host, a payload and `[]` reach the page.
- `tests/served_methods_browseros.rs` checks that the prelude's refusal of a method Neo does not serve is one row of the run's trace.
- `tests/main_world_browseros.rs` runs each main-world script API (`page.evaluate`, `waitForFunction`, `locator.evaluate`, `evaluateAll`, `elementHandle.evaluate`, `$eval` and `$$eval`, and the `frame` forms in a child frame) and `setContent` as Playwright does: its values, arguments, handles and errors, and the document `setContent` writes, whose inline script runs once, and whose password field reads as the page holds it in every read. An owner's lock that blocks reads withholds what a running script read. A unit test, `every_capability_op_has_a_browseros_case` in `src/builtins/op_marks.rs`, fails until every op that reaches the main world, Neo's files or Neo's screen names a test in one of these BrowserOS suites that proves it served or held.
- `tests/acceptance_browseros.rs` runs one task end to end. The agent drives a tab, and the host reads its trace, locks the tab, sees the agent's next write refused, releases the lock, and finds the tab's picture after that write.

`docker/smoke_sdk.py` checks a running claw-sdk Neo container from the host. It checks health and readiness, that `/mcp` lists the one `run` tool without issuing a session id, and that one run loads a page and reads its title. Run it as `python3 smoke_sdk.py --mcp-port <port>`. The port defaults to 9010.
