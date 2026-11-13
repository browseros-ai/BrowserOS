# Migrating from hosted `/v1` (claw-server) to the claw-sdk API

## Keep a tab while a task waits

Send `PUT /v1/tabs/{tabId}/keep` on the host listener when an approval wait starts. An absent
body or `{}` keeps the tab until you release it. Send `{"ttlSeconds":3600}` to end the keep
after an hour. The optional limit accepts 1 to 2,592,000 seconds. There is no cap without a
limit because a keep blocks nothing and only spares a tab from idle cleanup. It ends on server
restart, so a crashed host cannot hold anything that matters. Repeating PUT replaces the
previous limit, including replacing it with no limit. `GET /v1/tabs` includes `keep: {}` or
`keep: {"expiresAt": ...}` on each kept tab. `expiresAt` is Unix epoch milliseconds.

Send `DELETE /v1/tabs/{tabId}/keep` when the wait ends. DELETE succeeds even when the keep or
tab is already gone and never needs a browser connection. Release and expiry restart the idle
clock, giving the tab one full `tabs.idleCloseSeconds` interval. Closing the tab also ends its
keep, whether the owner, a script, or `task.close()` closes it. A keep permits every agent
operation. It only prevents idle cleanup. Only the host can set one, with its existing token.
No run, operation, or event is recorded. Each PUT and DELETE writes an info log line.

Replace periodic keepalive reads with these two calls. A keep is independent of settings,
locks, fits, and takeovers. Any tab a lock can name is accepted, including Neo's blank tab.
Internal, extension, and devtools tabs that locks cannot name return 404 `tab_not_found`, even
when the native inventory lists them. An absent tab
returns 404 `tab_not_found`; an older image with no keep route returns 404 `not_found`.

Keeps live in memory and end on server restart. After a server-only restart, preexisting tabs
are discovered as `Found`, so idle cleanup already leaves them alone. Persisting a keep could
instead protect the wrong tab after a browser restart reuses its number. If tab origins ever
survive server restart, keeps must become persistent in that same change.

## Trace summaries for an audit timeline

One request returns a page of runs with their operations, newest runs first and each run's
operations in start order:

```http
GET /v1/runs?taskId=checkout&limit=20&fields=summary&internal=false
```

Each `items[]` entry keeps its run metadata and adds `operations`. Follow `nextCursor` to read
older runs. An entry for a running run has its current operations, with no `outcome` until an
operation ends. `operationCount`, `complete`, and other execution facts describe the stored run,
including operations the view omits. Screenshot bytes stay at
`GET /v1/operations/{operationId}/screenshot`; `hasScreenshot` identifies available images.

`fields=summary` also works on `GET /v1/runs/{runId}` and `GET /v1/trace`. It omits arguments,
results, their truncation flags, call-log notes, host error codes, inline images, and the run's
source code, `codeWithheld`, host data, and downloads. Error owner and code remain; the required
error `message` is an empty string. Page URL, title, and tab identity remain exactly as recorded.
The paged audit view also omits operation `site` and `delivery`; detail and trace summaries keep
that structural metadata, including unknown-write identities on the run detail.

`internal=false` independently excludes Neo's own subscriptions before the trace tail limit.
Every operation read and operation event now carries `internal`, derived from the one shared
subscription-name rule used by screenshots and touched-tab following, plus an absent script
call site. Explicit script listeners such as `page.on` carry their site and stay in the view.
Historical listener rows without sites cannot distinguish script listeners from Neo's own work.

Default answers have two additive compatibility changes. Every operation row gains `internal`.
Newly recorded explicit `page.on` and `context.on` listener changes also gain their script `site`,
which the old prelude omitted. These sites appear in default run detail, single-operation reads,
trace tails, and operation events, even without either new query option. Neo's own listeners and
event pump keep no site. Existing stored rows are not rewritten, and their other fields retain
their values and presence. Explicit defaults are `fields=full&internal=true`. Summary ignores
screenshot-inlining options and leaves `hasScreenshot` intact.

This is a read view. Values remain stored in `trace.db`, and full reads, downloads, blobs, and
screenshot endpoints retain their existing access. URL and title can themselves contain values;
the host applies its own redaction to these audit fields.

## Frame trees Neo could not read whole

A script's `page.frames()` lists a tab's whole frame tree, cross-site frames included, from Neo's
own `page.frames` reads: before the script's first statement, and before each operation settles
for a tab whose frames changed. A read that could not get the whole tree ends with
`framesIncomplete: true`: an out-of-process frame's subtree would not read, or no tree came back
at all. A read that answered the whole tree ends with `false`. After a short read the script's
`frames()` lists the frames that were read, and the tab is read again before the script's next
operation settles.

Read the mark per operation. `true` says that one read was short; a later `page.frames` operation
of the same tab may have read it whole. `false` on a `page.frames` operation that failed (refused,
timed out) says nothing about the tree: it answered none, and the script kept the frames it had.

To find a run whose script was handed a short tree, read it and look for a `page.frames`
operation with `framesIncomplete: true`:

```http
GET /v1/runs/{runId}?fields=summary
```

These reads are Neo's own (`internal: true`), so `internal=false` leaves them out; keep the
default. `fields=summary` keeps the field. Operations recorded before it, and those whose end Neo
never recorded, have none. The server also logs each such read at warn, `a tab's frame tree is
not whole`, with the tab and the frame's error.

## Source layout and package names

The SDK's source packages now live together under `crates/neo-sdk/`, relative to
`packages/browseros-agent`. This changes source paths and package names, with no wire-format
or browser behavior change:

- `neo-server`: renamed from the inner `neo-sdk` crate (previously `claw-sdk` at `apps/claw-sdk`); the binary stays `browseros-claw-sdk`.
- `neo-playwright`: renamed from `browseros-playwright` at `crates/browseros-playwright`.
- `neo-playwright-js`: renamed from `@browseros/browseros-playwright-js` at `packages/browseros-playwright-js`.
- `neo-http`: renamed from `browseros-service` at `crates/browseros-service`, then from `neo-service`.
- `neo-takeover`: renamed from `browseros-takeover` at `crates/browseros-takeover`.
- `neo-trace`: renamed from `browseros-trace` at `crates/browseros-trace`.
- `neo-sdk-api`: renamed from `claw-sdk-api`; `contracts/claw-sdk-api` and `crates/claw-sdk-api` are colocated here.
- `neo-captcha`: renamed from `browseros-captcha` at `crates/browseros-captcha`; desktop uses the same solver.
- `neo-screencast`: renamed from `browseros-capture` at `crates/browseros-capture`, then from `neo-capture`; desktop uses the same capture hub.

Build the server with Cargo package `neo-server`. Release server kind `claw-sdk`, binary and
version marker `browseros-claw-sdk`, MCP server name and `CLAW_*` variables are unchanged.
Desktop's behavior and dependency graph are unchanged apart from the two shared crate names.
Tracing targets use `neo_captcha::…` and `neo_screencast::…` (`neo_capture::…` before the second
rename), and the host API's are `neo_http::…` (formerly `neo_service::…`); update custom `CLAW_LOG`
directives that name those crates. The default desktop log filter is unchanged.

## Host file paths

Scripts now read files by path only from `files.readDirs`, set through the host's
`GET`, `PUT` and `PATCH /v1/settings`. Its default is the browser downloads folder. Uploading
`download.path()` and copies saved inside that folder keeps working. Host code that stages
uploads elsewhere must add that absolute folder, or send `{ name, mimeType, buffer }` payloads.
The rule also covers `drop`, script/style tag paths and page/context init-script paths.

The state directory, browser profile, host token and CAPTCHA key files are always protected
from script reads and writes, even with `["/"]` listed. In hosted mode, so on the SDK image,
the home of Neo's user (`/home/neo`) is protected the same way. Inside it a script works only in
the downloads folder or in a listed folder whose first name below the home is not a dot-entry,
and there the list also allows writes. A script can no longer save under `/home/neo`: save under
`/data/downloads`, `/tmp` or a listed folder instead. A directory upload containing a
protected location is refused. Writes elsewhere stay unchanged. Relative paths use Neo's
working directory, symlinks resolve before authorization, and allowed aliases retain their
filenames. Empty lists deny path reads; missing listed folders grant no access. Refusals are
`script / invalid_input / not_sent` without disclosing the refused path.

Embedders can set `HostedOptions.read_dirs`. Omission uses the downloads default; an explicit
empty list denies path reads. Server launchers can set `CLAW_DOWNLOADS_DIR` and `CLAW_PROFILE_DIR`
to identify the browser's actual folders. The SDK image supplies both itself.

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
| `toolCalls[]` | `GET /v1/runs/{runId}`: `operations[]` | `parentId` links wrap retries and host calls; `tabId` is the tab each operation targeted. The run id is on the `run` result's `_meta["com.browseros.neo/run"].id` |
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
| `tabs.close_after_minutes = 5` | `PUT /v1/settings {"tabs":{"idleCloseSeconds":300}}` | Counts from the tab's last use, not from a session's end: a run in flight that used the tab, a lock, a fit, or a host keep on it all count as use. A popup an agent's tab opened is closed with it. Neo's own blank tab keeps the browser open |
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

## Neo's blank tab

Closing the browser's last tab quits the browser and Neo's server with it. Neo keeps one blank tab of
its own open at all times, so a host's tab reset no longer needs a keeper tab of its own: close every
tab the agent left, and Neo's tab keeps the browser alive.

- `GET /v1/tabs` lists it with `owner: "neo"`; every other tab has no `owner`. Locks, fits, takeovers and
  display follow treat it as any tab.
- It is the one exception to `context.pages()` listing every tab: no script sees it, no task takes it,
  and an op that names its tab fails `script / invalid_input` with nothing sent.
- Every close Neo makes opens it first if it is missing. If Neo cannot open it and the browser lists
  no other tab, `page.close()` fails `neo / cdp_error` with nothing sent, rather than quit the browser.
- After a browser restart Neo adopts a blank tab no task owns (the launch tab, or its own restored
  tab) before opening another. If the owner closes it, Neo opens another.
- Neo keeps it only while it is blank. Once the owner browses in it, or `POST /v1/fits/{id}/navigate`
  moves it, Neo opens a new blank tab and hands the old one back as an ordinary owner's tab: it loses
  `owner`, scripts list it (at once when they list tabs, else within 30 seconds), a context's `page`
  listeners hear of it as a new page, and the janitor never closes it.

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

## Typing a host's secret

zuzu's own entry scripts (`fill.js`, `login_fill.js`, `code_entry.js`, `focus_box.js`), sent as
`run` code with the secret inside, become one host call. The value never enters a script, a
record or a picture:

```http
POST /v1/tabs/{tabId}/fill
{"requestId": "fill-7f3a", "taskId": "checkout", "url": "https://pay.example/card",
 "fields": [
   {"ref": "f2e4", "value": "4242424242424242", "spaced": true,
    "expect": {"origin": "https://pay.example", "autocomplete": "cc-number"}},
   {"ref": "f2e6", "value": "123", "expect": {"origin": "https://pay.example"}}
 ]}
```

- **Name the box by ref**, as the agent's snapshot printed it or `locator.ref()` minted it, never
  by selector. `expect.origin` is required: the box's own frame must be on it. Every other fact
  sent in `expect` must still hold when Neo looks (`box_changed`).
- **Look first, without a value**: `POST /v1/tabs/{tabId}/fill/check` takes the same fields
  with no `value` and answers each box's facts and the first refusal a fill would meet now. It
  replaces `fields.js` and the box checks in `auth_fields.go`; zuzu's rule for which box may take
  which secret stays in Go, on these facts. Send the facts you approved back as `expect` on the
  fill, so the look at approval and the look at typing are one rule. It takes `lockId` as the
  fill does.
- **Read `outcome` first.** Every answer about the page is a 200: `entered`,
  `entered_page_moved` (the form may have submitted), `refused` (no character went out),
  `partial` (boxes before `at` hold their values, box `at` none) or `uncertain` (look at the page
  before repeating). `error` is the operation's error, with the route's codes listed in the
  contract; `typed` counts characters per box, never text.
- **A `requestId` types once**, across a restart too. A repeat while the first runs waits for
  its answer; after it, it gets the stored answer and types nothing. A refused fill's id is
  spent; only a request error (400, 404, 409, 503) leaves it free. After a restart in the middle
  of a fill, its id answers `uncertain` with `fill_interrupted`.
- **Locks.** A lock on the tab refuses the fill as any write. To type through your own lock,
  send its id as `lockId`; only a lock the host took with `POST /v1/locks` on that tab passes. A
  takeover's lock refuses, named or not. A named lock no longer held is `409 lock_released`.
- **Host only.** The host token on the host listener. A request a web page sent (`Origin` or
  `Sec-Fetch-Site`) is `403 forbidden`, and no script can name the methods.
- **The record.** The fill is a run of Neo's own with one operation, `host.fill` (kind
  `protected`), whose `args` name the boxes and each value's length. It records no page address,
  no title and no call-log notes. Drop the trace exclusion of scenario `88-secret-canary`.
- **No pictures of that tab again.** From the fill on, Neo takes no screenshot of the tab for
  the rest of its life: going back in history restores a plain box's value into a new document.
  Its operations keep `hasScreenshot: false`, and their screenshot answers `404 not_found`. A
  script's own `page.screenshot()`, `locator.screenshot()` and `page.pdf()` of the tab fail
  `host / private_tab` with nothing sent. It is permanent for that tab, and the message the
  agent reads says trying again will not succeed. `retryable: true` on any error means a repeat
  is safe, never that it will succeed.
- **Bullets for the boxes Neo typed into.** Every read Neo makes of such a box answers one `•`
  per character, whatever the page declares: the snapshot, `locator.inputValue()`, and every
  assertion (the received value and aria tree, the preview and the failure's snapshot), so
  `toHaveValue` compares against the bullets. A script's own code in the page (`evaluate`) still
  reads the value; zuzu's redactor for its own secrets stays.
- **Two gaps, until later masking work.** An element preview in a call log or error (`locator
  resolved to <input …>`) prints the box's attributes unmasked, so a page that writes the typed
  value into the `value` attribute shows it in a failed assertion or action on that box. And
  the mark stays in the box's frame: a box the page moves into another same-origin frame reads
  its value there from `inputValue()` and assertions.
- **Not covered**: what the owner types in a takeover or on the display, a script's own fill
  and the browser's autofill; a box the page re-creates, or a value Back restores, has no mark.
  A Neo restart keeps which tabs took a fill (a synced record beside the trace). `neo.solveCaptcha`
  still photographs its challenge. A dropdown (`expiry_select.js` stays), a link with a token in
  its address (`open_link.js` stays), and the browser's saved form state, which can bring a
  value back after a restore (the host clears it). The contract's fill route lists each limit.
- **A script's own fill of a card number** is not this route, and its pictures are withheld
  anyway: Neo stores no picture of a tab while any frame of it shows a box masking treats as
  private, not empty and painted in clear. A password box paints bullets, so a sign-in page keeps
  its pictures.

## Error bodies

The body shape is unchanged: `{code, message, requestId}`. The codes are new
(`unauthorized`, `expired`, `tab_not_found`, `lock_released`, `fit_restored`,
`takeover_released`, `forbidden`, `trace_unavailable`, …). As before, clients must accept
codes they do not know.

## Removed privacy endpoints and fields

- Stop calling `POST /v1/tabs/{tabId}/private`, `POST /v1/tabs/{tabId}/sweep` and
  `POST /v1/secrets`. Use `POST /v1/locks` or the takeover flow to block agent actions, and
  `POST /v1/tabs/{tabId}/fill` to type a secret (see "Typing a host's secret").
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

## Values omitted by the host

`trace.values` defaults to `on`. With `off`, each operation and run that ends omits script
text, arguments, results, notes, host data and stored replies. Errors retain their owner, code,
host code, retryability and operation id, with an empty message. `Run`, `RunSummary` and
`Operation` add the optional open-list marker `values: "off"`; absent means normal recording.
A run can contain operations recorded under different settings. HTTP reads and events expose
the stored record. Values already stored are unchanged.

The live reply and the ten-minute in-memory request replay are unchanged. After a restart,
a repeat of an off run gets `duplicate_request` even inside that window. It never executes
twice while its request history is retained.

Result blobs written with values off expire ten minutes after their write. Neo writes the
absolute expiry in a sidecar before the first blob, restores it at startup, and deletes due
blobs on minute upkeep. This also covers a crash before the trace end commits. Downloads,
page metadata and Neo's separately configured screenshots retain their existing rules.
The store adds one migration; an older build refuses the newer schema and starts fresh.
