# Migrating from hosted `/v1` (claw-server) to the claw-sdk API

## Source layout and package names

The SDK's source packages now live together under `crates/neo-sdk/`, relative to
`packages/browseros-agent`. This changes source paths and package names, with no wire-format
or browser behavior change:

- `neo-server`: renamed from the inner `neo-sdk` crate (previously `claw-sdk` at `apps/claw-sdk`); the binary stays `browseros-claw-sdk`.
- `neo-playwright`: renamed from `browseros-playwright` at `crates/browseros-playwright`.
- `neo-playwright-js`: renamed from `@browseros/browseros-playwright-js` at `packages/browseros-playwright-js`.
- `neo-service`: renamed from `browseros-service` at `crates/browseros-service`.
- `neo-takeover`: renamed from `browseros-takeover` at `crates/browseros-takeover`.
- `neo-trace`: renamed from `browseros-trace` at `crates/browseros-trace`.
- `neo-sdk-api`: renamed from `claw-sdk-api`; `contracts/claw-sdk-api` and `crates/claw-sdk-api` are colocated here.
- `neo-captcha`: renamed from `browseros-captcha` at `crates/browseros-captcha`; desktop uses the same solver.
- `neo-capture`: renamed from `browseros-capture` at `crates/browseros-capture`; desktop uses the same capture hub.

Build the server with Cargo package `neo-server`. Release server kind `claw-sdk`, binary and
version marker `browseros-claw-sdk`, MCP server name and `CLAW_*` variables are unchanged.
Desktop's behavior and dependency graph are unchanged apart from the two shared crate names.
Tracing targets use `neo_captcha::…` and `neo_capture::…`; update custom `CLAW_LOG` directives
that name those crates. The default desktop log filter is unchanged.

## Hosted API migration

When the Neo Docker image runs claw-sdk instead of claw-server, the hosted `/v1` API on port
9010 goes away. There is no compatibility shim, since zuzu is the only client. This note maps
every old call to its replacement in [`openapi.yaml`](openapi.yaml).

## What changes for every call

| | hosted `/v1` (claw-server) | claw-sdk API |
|---|---|---|
| Where | `http://127.0.0.1:9010/v1/…`, the same port as `/mcp` | the host listener: the Unix socket `CLAW_HOST_SOCKET` (default `$BROWSERCLAW_DIR/host.sock`), or `CLAW_HOST_TCP` when set. Never the MCP port |
| Auth | none | `Authorization: Bearer <token>`, with the token from the file `CLAW_HOST_TOKEN_FILE` names; on the socket, the peer uid too |
| Sessions | every call names a session | none. Neo issues no session id. Locks belong to a browser tab (`tabId`), and you group a task's runs with your own `taskId` |
| Unit of work | a session's tool calls (`toolCalls[]`) | runs, each with an operation tree (`runs[]`, `operations[]`) |
| Failure | `outcome: error` | `outcome` plus `delivery` (`not_sent` / `done` / `unknown`) and `error.owner` (`neo` / `script` / `website` / `host`) |

Go dials the socket with a custom `DialContext` on its `http.Transport`. The URL host is then
ignored, so `http://neo/v1/…` works.

Send `_meta["com.browseros.neo/run"] = {requestId?, taskId?, hostData?}` with each `run`. All
three are optional, and the two ids are at most 128 bytes each:

- `requestId` is your id for the call (zuzu's tool-call id). A repeat of it never runs again,
  within its task when the run has a `taskId`, else across Neo.
- `taskId` is your id for the calls of one task. It replaces the session for grouping. You
  list a task's runs, trace and events by it, and the agent's `page` starts on the task's last
  tab. An unknown host `taskId` is accepted; it registers a Neo task when it first opens a tab.
- `hostData` is stored with the run, bounded for storage, and Neo never reads it.

## Tasks

Each run starts with fresh JavaScript. Hosts that send `taskId` seed its current task;
the neo-viewer proxy is unchanged. Host ids and `neo.task(id)` use the same task records,
and hosts retain the last-used-tab fallback from memory or the trace store.
With no host or script task, built-in `page` opens a new tab on first use, every run;
`context.newPage()` opens another. Both tabs have no task; neither reuses an existing tab.

For work across runs, the agent creates a task with `await neo.createTask(name)`, returns
`task.id`, then starts later runs with `await neo.task(id)`. The latest task pick wins.
Built-in `page` uses that task's last-used open tab, else another open tab, else a new tab
in it. `neo.task(id)` binds it to that open tab at once; without one, or when the run already
holds that tab's Page, it picks at its first action (until then `page.tabId` and `page.taskId`
are null), and it keeps its first tab for the run. `context.newPage()` opens in the current task; `task.newPage()` opens in that task.
Popups join their opener's task. `task.pages()` lists its tabs oldest first; `task.page(tabId)`
picks one. `await task.close()` closes its tabs and ends it. A local `const page` is allowed.
The [agent handout](../agent.d.ts) has the four-run walkthrough.

Any script may use any tab or task. `context.pages()` lists every tab, each with `tabId`
and `taskId` (`null` without a task). Recover a lost id with
`context.pages().find(p => p.url().includes('ycombinator'))?.taskId`.
An unknown id throws `task_not_found`, whose hint points to `context.pages()` and `page.taskId`.

Tasks live in memory, at most 4096. A Neo restart forgets them while tabs stay; the tabs
remain reachable through `context.pages()` without task ownership. Host tasks retain their
trace fallback. The janitor closes agent-opened tabs nobody used for `tabs.idleCloseSeconds` (30 minutes unless the host sets it; see Settings below).

Hosts get touched tasks in `_meta["com.browseros.neo/run"].tasks`, always present (`[]` when
empty). Each entry has `id`, `name`, `kind`, and `tabs: { tabId, url }[]`. Tasks appear once,
in first-touch order, even when the script failed. Tabs and URLs reflect reply time; closed
tasks have kind `closed` and no tabs. `structuredContent` is unchanged.

The agent's reply is exactly what the script returned, plus console lines and existing notes.
Task metadata adds no reply text; the agent must return `task.id` to keep it.

## Call by call

| hosted `/v1` | claw-sdk API | Notes |
|---|---|---|
| `GET /v1/sessions?client=&since=&until=&q=&cursor=&limit=` | `GET /v1/runs?taskId=&since=&until=&cursor=&limit=` | There are no sessions to list. A task's runs come back newest first. `q` and `client` filters are gone; `name`, `site` and `summary` are not recorded |
| `GET /v1/sessions/{id}` (`status`, `toolCalls[].page`) | `GET /v1/trace?taskId={taskId}&tail=20` | Nothing is live or ended without a session; each run has its own `status`. zuzu's `CurrentTab` becomes the newest operation's `tabId` in the tail. `GET /v1/runs?tab={tabId}` lists the runs that acted on one tab |
| `GET /v1/sessions/{id}?include=screenshots` (zuzu's `Audit`) | `GET /v1/trace?taskId={taskId}&tail=20&screenshots=last:4` | `tail` and `screenshots=last:N` replace `trimNeoAudit`. Add `tab={tabId}` to keep only the operations on one tab |
| `GET /v1/sessions/{id}?wait=60` | `GET /v1/runs/{runId}?wait=60` | Completeness is per run (`complete`), not per session |
| `toolCalls[]` | `GET /v1/runs/{runId}`: `operations[]` | `parentId` links wrap retries, host calls and protected fills; `tabId` is the tab each operation targeted. The run id is on the `run` result's `_meta["com.browseros.neo/run"].id` |
| — (zuzu's before/after audit diff) | `GET /v1/runs?requestId={toolCallId}&taskId={taskId}` | With a `requestId`, a lookup after a transport timeout says whether the run started and what it sent. Neo always returns its own run id |
| `GET /v1/sessions/{id}/recording` (rrweb) | — | Recording is deferred. The old endpoint returned no pages in the Docker image anyway, because the bundled extensions are off |
| `GET /v1/sessions/{id}/screencast?quality=&tab=` | `GET /v1/tabs/{tabId}/screencast?quality=` | The tab is in the path; there is no stream that follows a session's current tab. Read parts by `Content-Length`, as today. The stream ends when the tab closes. `tab.closed` on `/v1/events` replaces the five-second `Live()` poll |
| `GET /v1/sessions/{id}/video?quality=&tab=` | `GET /v1/tabs/{tabId}/video?quality=` | As for the screencast |
| `GET /v1/events?session=` | `GET /v1/events?taskId=&tab=` | `session.*` events are gone. `tab.closed` is new, and run and operation events carry `taskId` and `tabId`. Resume with `Last-Event-ID`; `stream.reset` means events were missed, so re-read the state endpoints |
| — (Xvnc on 127.0.0.1:5900, no auth) | `GET /v1/display?mode=view\|control&followTab={tabId}` or `&followTask={taskId}` | RFB over WebSocket. `view` drops input; `followTab` or `followTask` (not both) replaces zuzu's `follow`/`toFront`; `control` forwards input |
| — (zuzu's handoff: `frontPage`, `fitOwnerView`, `giveBack`, `steerOwnerTab`) | `POST /v1/takeovers {tabId, viewer?, reads?, ttlSeconds?}` when the owner opens the link, `POST /v1/takeovers/{id}/release` when they are done | The flow composes small parts a host can also use alone: `POST /v1/locks {tabId, reads?, ttlSeconds}`, `POST /v1/tabs/{tabId}/fit` (+ `GET /v1/fits/{id}`, `…/navigate`, `…/restore`), and `/v1/display`. Neo locks the agent out of the tab (`host / owner_has_control`) and fits the page to the owner's screen. `reads: block` is the default; send `allow` to let the agent keep reading the tab. To pause a whole task, stop sending its runs |

## Settings

The hosted server read its behaviour from `neo.toml` (`tabs.grouping`, `tabs.close_after_minutes`,
`run.wall_timeout_seconds`, `scripts.isolated_world`). claw-sdk reads no file. A host sets it through
`GET`, `PUT` and `PATCH /v1/settings` instead, at the start step that runs before its first run:

| `neo.toml` | claw-sdk | Notes |
|---|---|---|
| `tabs.close_after_minutes = 5` | `PUT /v1/settings {"tabs":{"idleCloseSeconds":300}}` | Counts from the tab's last use, not from a session's end: a run in flight that used the tab, a lock or a fit on it all count as use. A popup an agent's tab opened is closed with it. The browser's last tab is never closed |
| `run.wall_timeout_seconds = 120` | each `run` call's `timeout` (`120000`) | Per call, not a setting |
| `tabs.grouping`, `scripts.isolated_world` | — | No tab groups; Neo's own page scripts always run in the isolated world |
| — | `trace.retentionSeconds` | How long the trace keeps a run, 7 days by default and at least an hour |
| — | `trace.screenshots` | `off` stops Neo's own screenshots; a script's `page.screenshot()` is kept either way |
| — | `trace.valueBytes` | How much of each value the trace keeps whole; replaces the environment value `CLAW_TRACE_VALUE_BYTES` (#3517) |

A name Neo does not know is skipped and listed in the answer's `unknown`, never refused, so a host can send
its whole list to any server version. A value of an open list Neo does not have yet is skipped the same way
and its setting named in `unknownValues`. A value out of range is refused with 400, naming the setting, and
nothing applies. Neo saves what the host set under `/data` and loads it at every start, a restart of the
server alone and a new container on the same `/data` alike. Send the `PUT` at every start of the host
anyway, so the saved set is what the host wants now and not what an older host or a hand `PATCH` left. A
server from before the settings answers 404.

## Downloads

zuzu's `neo_download` → `neo_upload` and `import` flows move into the script and onto the run's
record. A script waits for the file with Playwright's own calls:

```js
const [download] = await Promise.all([page.waitForEvent('download'), page.getByRef(ref).click()]);
return { path: await download.path(), filename: download.suggestedFilename() };
```

- `download.path()` resolves once the browser finished the file and returns where the browser
  saved it (`/data/downloads/…` in the image, ` (1)` added on a clash). That path is on Neo's
  disk: a later run uploads it with `locator.setInputFiles(path)`. `download.saveAs(path)` copies
  it, and `download.failure()` reads `null` or why it failed (`canceled`).
- `GET /v1/runs/{runId}` lists the run's `downloads[]`: `index`, `url`, `suggestedFilename`,
  `path`, `size`, `state` (`completed`, `in_progress`, `failed`) and `failure`. A download is
  the run's once one of its `download` events handed it to the script, whether or not the script
  asked for the file.
- `GET /v1/runs/{runId}/downloads/{index}` returns the bytes of a `completed` one
  (`application/octet-stream`, the suggested name in `Content-Disposition`), on the host socket
  with the same token. They are a copy Neo took when the run ended, so they are there as soon as
  the run's reply is, and they go when retention removes the run (7 days). This replaces reading
  the file through `read_output` parts: `/data` stays `neo 0700`.
- A download still being written when the run ended is recorded `in_progress` with no bytes.
  Await `download.path()` before returning when the host needs the file.

## Large results

A `run` reply carries its value inline, at most 256 KiB of JSON. zuzu's `read_output` route for
bigger outputs (full-page screenshots, PDFs, `DecodeRun`'s fallback) becomes blobs:

- When a returned value would be over the limit, Neo keeps its largest byte values (a `Buffer`
  or `Uint8Array`, which travel as `{"$neo":"bytes","base64"}`) with the run, largest first until
  the value fits, and puts `{"$neo":"blob","id":"b0","size":N}` in each one's place. Byte values
  in a reply that fits stay inline, as before.
- Read each at `GET /v1/runs/{runId}/blobs/{id}` on the host socket, with the same token
  (`application/octet-stream`, `size` bytes). The run id is the reply's
  `_meta["com.browseros.neo/run"].id`. A blob is there as soon as the reply is, and it goes when
  retention removes the run.
- Text is never kept as a blob. A value that still does not fit fails `script / result_too_large`,
  whose message says to return large text as `Buffer.from(text)`.
- `GET /v1/runs/{runId}` shows the same markers in `result`, and a repeated `requestId` replays
  them.

```js
const shot = await page.screenshot({ type: 'png', fullPage: true });
return { shot };   // {"shot": {"$neo":"blob","id":"b0","size":1505153}} when it is over the limit
```

## Error bodies

The body shape is unchanged: `{code, message, requestId}`. The codes are new
(`unauthorized`, `expired`, `tab_not_found`, `lock_released`, `fit_restored`,
`takeover_released`, …). As before, clients must accept
codes they do not know.

## Removed privacy endpoints and fields

- Stop calling `POST /v1/tabs/{tabId}/private`, `POST /v1/tabs/{tabId}/sweep` and
  `POST /v1/secrets`. Use `POST /v1/locks` or the takeover flow to block agent actions.
- Stop subscribing to `tab.private` or reading `Event.privacy` and `TraceTail.visuals`.
  Use each operation's `hasScreenshot` to check for a stored image.
- Drop handlers for the host API codes `page_private`, `withheld` and `privacy_unavailable`.
  An old trace that withheld a screenshot answers `404 not_found` for that image.
  An image removed by retention still answers `410 expired`.
- `Run.codeWithheld` only reports `unavailable` for a run interrupted by a restart. An older
  run recorded as `sensitive_input` can still have no stored `code`.

## Trace arguments and sensitive targets

Operation arguments no longer replace fields with `[redacted]` based on method metadata or a
password, card or one-time-code target. Typing into these fields no longer withholds the run's
code as `sensitive_input`. Arguments and code still wait for their end record before storage,
with the existing size bounds. `Run.codeWithheld: unavailable` still marks
code that was never stored, for example after a restart.

Existing trace rows remain readable: `[redacted]` arguments stay as recorded, and an older
run whose code was withheld still has no code. No storage migration or reconstruction is done.
