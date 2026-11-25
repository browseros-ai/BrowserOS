# Neo viewer

A local tool for one person: start it, give an agent its one MCP URL, and watch every `run()` call
the agent makes against the Neo SDK image: the code it sent, the methods each line called, what the
model read back, how long each took and roughly how many tokens, and Neo's screen live. It is a dev
tool under `tools/`: not shipped, and not part of the image.

![The viewer with a run open](docs/screen.png)

```text
 agent ──MCP──▶ viewer (Bun, 127.0.0.1:7310/mcp) ──MCP──▶ Neo SDK image (127.0.0.1:<port>/mcp)
                  │ records every call and reply,       ▲ │
                  │ as sent and as received             │ │ host API: run records, metrics, live screen, takeovers
                  ▼                                     │ ▼ OpenTelemetry push (OTLP/HTTP JSON) to /otlp
               log.jsonl in the state folder ──▶ /api/* ──▶ the page at /
```

## Start it

From `packages/browseros-agent`:

```sh
bun run neo:viewer
```

That is the whole loop. It works out this checkout's Neo SDK image the way `build.sh` names it
(`neo-sdk:<browser>-<commit>[-dirty]-<arch>`), builds it with `build.sh` when it is not on the
machine or the checkout's uncommitted build inputs changed since the viewer built it, starts it
fresh, and prints:

```text
  Viewer:         http://127.0.0.1:7310
  Agent MCP URL:  http://127.0.0.1:7310/mcp
  Claude Code:    claude mcp add --transport http neo-viewer http://127.0.0.1:7310/mcp
```

Open the Viewer URL. Give every agent the same MCP URL: there is one, and every agent's runs go
into one list. A first build takes about 7 minutes; a new commit that leaves the server alone
rebuilds from Docker's cache in seconds, and uncommitted changes outside the image's build inputs
(this tool, say) reuse the commit's image.

Ctrl-C stops the viewer and removes the container and network it started. Each start begins
clean: it removes what the previous start of the same instance left (the container, network and
data) and, after a new build, the image the previous build made.

| Flag | |
|---|---|
| `--name <instance>` | which viewer this is (default `default`); see Instances |
| `--keep-data` | keep the browser profile (logins) and the run list between starts |
| `--no-build` | use the newest `neo-sdk` image already on this machine |
| `--image <tag>` | use this image; nothing is built |
| `--upstream <url>` | forward to a Neo MCP endpoint you started yourself; no container |
| `--host-token-file <file>`, `--host-api <url>` | with `--upstream`: that Neo's host API (`CLAW_HOST_TCP`, default `http://127.0.0.1:9300`), for run records and the live screen |
| `--port <port>` | the viewer's port (7310) |
| `--data-dir <dir>` | the viewer's state folder (see The log) |

Pass flags after `--`: `bun run neo:viewer -- --keep-data`. The same settings come from
`NEO_VIEWER_INSTANCE`, `NEO_VIEWER_PORT`, `NEO_VIEWER_UPSTREAM` and `NEO_VIEWER_DATA_DIR`. The
viewer listens on 127.0.0.1 only, and publishes the image's ports on 127.0.0.1 only.

### When it stops at the packages

Before any Docker work it checks this tool's packages against its `package.json` and `bun.lock`
(missing, or not the lockfile's version after a pull) and builds the page, so a bad install fails
in a second. When something is missing it installs it with the Bun that is running (`bun install
--frozen-lockfile`, so `bun.lock` is never rewritten), says so in one line, looks again and
carries on:

```text
Installing the viewer's packages with Bun 1.4.4 (acorn missing): bun install --frozen-lockfile in …/packages/browseros-agent
```

Any Bun from 1.2.19 does this (`MIN_INSTALL_BUN` in `server/dependencies.ts` says what was
proved). An older Bun, or an install that still left something missing, stops with one line naming
the Bun needed and the command to run:

```text
neo-viewer: its packages are missing or older than bun.lock (acorn missing), and Bun 1.2.18 cannot install them: it needs Bun 1.2.19 or newer. Run: cd …/packages/browseros-agent && npx -y bun@1.4.2 install --frozen-lockfile
```

### Instances

Every start belongs to one instance, `--name <instance>` (or `NEO_VIEWER_INSTANCE`, default
`default`): the container `neo-viewer-<instance>`, the network `neo-viewer-<instance>-net`, the
volume `neo-viewer-<instance>-data` and the state folder `~/.local/state/neo-viewer/<instance>`.
A start replaces only an earlier start of the same instance; if that one's viewer is still running
it says so in one line, and refuses (touching nothing) when that viewer holds the same port.
Cleanup never touches another instance: each object carries the labels `com.browseros.neo-viewer`
and `com.browseros.neo-viewer.instance`, a same-named object without them stops the start, and the
only images removed are ones this instance's own builds replaced (recorded in its `start-state.json`).
Instances on one commit share one image, so even that waits while anything else holds it: any
container, running or stopped, or another instance's `start-state.json` naming it as its last build.
The start says in one line what it kept and why, and tries again at its later starts; it never
forces a removal (Docker refuses a plain one under a container, and a forced one deletes the image
under a stopped container).
**Workers and scripts: always pass your own `--name` and port; `default` is the owner's.**

### The container

It is started as [the image's README](../../crates/neo-sdk/neo-server/docker/README.md) shows for
a standalone start: its own Docker network, `NEO_MCP_REMOTE=true`, and the host API over the
development option `CLAW_HOST_TCP` with a token file the command writes (mode 0600) in the state
folder; both ports are published on 127.0.0.1 at ports Docker picks. Once the container is
healthy, the viewer sends `PUT /v1/settings {"trace":{"valueBytes":262144}}` through the host API
(`server/neo-settings.ts`), so Neo's trace keeps each call's arguments and result whole up to Neo's
own reply limit rather than its 2 KiB default: the right panel shows them whole. Neo keeps the
setting under the container's `/data`; with `--upstream` the viewer sets nothing, since the server
is someone else's. An image older than the setting gets a warning and keeps its cut values. Inside the container
`CLAW_HOST_TCP` binds `0.0.0.0:9300`: bound to `127.0.0.1` the published port answers nothing,
since Docker's port forward arrives from outside the container's loopback. When Docker's address
pools are used up it names a /24 itself and says so. The command is the only control: the page
shows the image and whether the container is up, read-only.

The container also pushes its OpenTelemetry to the viewer (see Neo's telemetry):
`OTEL_EXPORTER_OTLP_ENDPOINT=http://host.docker.internal:<viewer port>/otlp`,
`OTEL_EXPORTER_OTLP_PROTOCOL=http/json`, `OTEL_BSP_SCHEDULE_DELAY=500`,
`OTEL_METRIC_EXPORT_INTERVAL=500` and `--add-host=host.docker.internal:host-gateway`; an image older
than telemetry ignores them.

## The screen

The page at `/` (the approved design, "Viewer C v2"):

- **Top bar.** Whether the container is running and its image tag (read-only; with `--upstream`
  it says the image is managed outside the viewer and where MCP goes), the MCP URL with **Copy**,
  **Clear**, **Export** (below), and **Take over** (below). Once Neo's telemetry arrives (see
  Neo's telemetry), Neo's health follows the container line: runs in flight, the run and
  operation error rates and renderer crashes since the last Clear, tabs open, and how fresh
  that is (**live**, `7 s ago`, or **no metrics** once no source has answered for 30 s). An image
  older than telemetry has no health line.
- **Runs, Metrics and Traces.** Runs opens by default. Metrics shows operation outcomes and
  duration buckets since Neo started. In **Slow operations**, choose a method and bucket to
  search the recorded spans for calls in that range. Metrics can include calls whose spans
  the viewer never received or no longer retains.
- **Traces.** Search by trace ID and inspect the span waterfall and attributes. The list shows
  the browser work for each recorded run. Several runs can belong to the same agent trace. Choose **Load more traces** to see older matches.
  **Open in Runs** opens the corresponding run and selects the recorded call when available.
- **Neo health.** Clicking the health opens Neo's metrics in place of the runs (the top bar,
  the strip and the sidebar keep streaming; **← Runs** or Escape go back to the runs as they
  were): runs by status, each operation's calls, failures, p50 and p95 with its duration
  histogram (one column per bucket of Neo's own bounds, so reading down shows which op is slow),
  navigations by HTTP status (a 4xx or 5xx in red, though its goto settled ok) and the ones the
  network refused by net error, CAPTCHA solves by family and verdict, by reason and what
  providers charged, errors by owner and code, tabs open and opened by origin and closed by reason
  with their lifetimes, renderer crashes, and every metric family in Prometheus' text form,
  folded. These counts are Neo's own since it started.
- **Where the time went.** The wall clock since the last Clear, from the first call to the last
  reply (to now while a run is running): each run sits where it happened, its time in Neo in blue
  (red if it failed, amber while running) after a grey sliver for the time the viewer held the call.
  On the left, the total in Neo against the time between calls (the model thinking, the agent's
  other tools, any human wait).
- **The runs.** One row per `run()` call, in order: when it started, the client that sent it,
  its first statement, how many calls Neo recorded, Neo time with a bar, the round trip the
  agent waited, **Code tok** (the script as sent, what the model wrote) and **Reply tok** (what came back,
  what the model read) with a stacked bar, and a totals row. Every bar is a share of its column's
  largest value. Click #, Neo time, Round trip, Code tok or Reply tok to sort by it (again to
  flip). Tokens are estimates, characters ÷ 4, always shown with "≈".
- **An open run.** Click a row: its script laid out one statement per line, indented by depth,
  with the line numbers counting those lines. Agents send code minified on one line; only the
  whitespace between tokens changes (`shared/code-format.ts`, driven by acorn's parse), so every
  token is as sent, a line still wider than 80 columns breaks at an object, a method chain or an
  argument list, and code that does not parse is shown as sent. On each line, the methods that
  line called, from Neo's record of the run: start offset, the name Neo recorded (it
  can differ from what the script wrote: `page.textContent('h1')` is `locator.textContent`), the
  outcome, a bar for when it ran within the run's own time, its duration, and read or write. The
  **Start** column is when the call started, measured from the run's start (its header and each
  value say so on hover). A line in a loop runs more than once: its first row counts the calls
  (`×3`) and each later row says which call it is (`2nd call`) where the code would be; a line
  that called many folds the rest behind one row. Calls are counted by their place in the script,
  and Neo's record does not mark where one call ends, so a call that makes several operations
  from one place (`page.waitForURL` checking again) counts each, as a loop's passes would. Neo's own bookkeeping (`page.snapshotPending`,
  `events.listen`, the row of operations not on a line) says **Neo internal**, so it is not taken
  for the script's calls. Click a method to select it: its arguments (a locator by its text, the
  timeout, the name the script used when it differs, a long string by its start), call log and
  outcome open under it, and what it returned shows on the right. Under the code, **Model read**:
  the reply in one line, its size, and **Open full**; clicking **Model read** shows the reply on
  the right again. A run still running, or one without Neo's record, shows its code alone. With
  no host API (`--upstream`) the record is folded from Neo's spans: the same rows, and one line
  saying spans carry no arguments, call logs or results. Once Neo's spans reach the viewer, a
  **Trace** row follows: the run's trace id, its spans, what failed and how long it took.
- **The right panel.** The sidebar's lower half shows the selection. A selected call shows what
  it returned, titled where it sits (`page.ariaSnapshot · run 8 · line 7 · call 2 of 3`), with
  its size. **Formatted** is what the script's `await` got wherever that can be recovered: a
  snapshot's text as the operation answered it when Rust printed it, or, when the run rendered
  it, a whole-page snapshot's text recorded by the `page.snapshotPending` after it; an `evaluate`
  value decoded from the page's serialized form, a string as lines, and `mode: 'default'`'s
  YAML rebuilt from the recorded tree as the run builds it; a locator's v1 snapshot (its options
  never reach the wire) shows the recorded tree, and any other result Neo's record of it, and the
  panel says which. **Raw** is Neo's record of the result, as JSON. Otherwise the panel is
  **What the model read**: the selected run's whole reply, exactly as the agent got it,
  formatted (keys aligned, a snapshot line by line) or raw. Until you click a row it follows the
  newest run. Drag the handle between the runs and the sidebar to resize it. **Trace** shows the
  run's OpenTelemetry waterfall there: `neo.run`, its operations' spans under it (one that ran
  inside another a step further in), each a bar on the run's time with its duration, red when it
  failed (as Neo says it: `neo.op.outcome` `error`; a cancelled op is not a failure). Neo's own
  work for the run (operations with no source line: `page.frames`, `events.listen`, the
  `events.next` long-poll, opening the task's page) folds under one quiet **Neo internal** row
  after the script's calls, as in the run block. Beside it, the picked span's attributes exactly
  as Neo emitted them, with whether any span
  carries site detail (`url.*`), which is how redaction is checked. A span and its method row are
  one operation (`neo.op.id`): picking a span opens its call in the block, and a call picked
  while the trace shows marks its span.
- **Open full.** The selection in a large pop-up with line numbers, find (Enter and Shift+Enter
  step through the matches), copy, and previous / next: the run's other calls for a call, the
  other runs' outputs for a reply. Escape closes it. A trace's is the waterfall wide, the span's
  attributes and Neo's resource beside it, and previous / next span (or ↑ ↓).
- **The live screen.** The sidebar's upper half: Neo's screen, live. **Follow agent** watches the
  tab the agent's latest run used and moves with it; **Control** lets you click and type (the agent
  is not held). Docked, it is as wide as the sidebar (drag the handle to make it bigger) and keeps
  Neo's screen ratio. **Pop out** opens it large over the page; closing it docks it back. **Take over**
  (top bar, or in the pop-up) gives you the agent's tab: its calls on that tab fail with
  `owner_has_control` until you press **Give back**, or until the takeover expires (Neo's default,
  20 minutes). The line under the screen says what your clicks do. While following, it says
  **Following is behind** once no status check has confirmed the agent's tab for over 5 s, since
  the screen may then still show an older tab; it goes back as soon as one answers.

`/?plain` is a plain list of the same runs (code, reply, raw request and reply, the live screen
beside it), kept for debugging.

## What the proxy changes

One header at most, and only one way. Every method, path, header and body reaches Neo as the agent
sent it, and every reply reaches the agent as Neo sent it, byte for byte, chunk by chunk as it
streams. The one addition: a request whose single run call carries no W3C trace context gets a
`traceparent` header the viewer mints (`00-<trace>-<span>-01`, sampled), so Neo's `neo.run` span
hangs under a span id that is that run's alone (`server/proxy/trace-context.ts`). A caller's own
`traceparent`, as a header or in `_meta["com.browseros.neo/run"]`, is never replaced or added to
(a header would override the `_meta` one), and a JSON-RPC batch of several calls gets none. The run
keeps its context either way (`RunSummary.trace`: trace id, span id, `injected`). The body is never
touched; so
Neo behaves exactly as it does without the viewer: a run that names no task gets a new tab of its
own (plain Playwright), a script picks a task with `neo.createTask`/`neo.task(id)`, and a request's
`_meta["com.browseros.neo/run"].taskId` (a host's task) keeps that task's tab (#3452).
That `_meta` taskId passes through untouched and the run shows it beside the client
(`RunSummary.agentTaskId`). The tasks a run touched come back in the reply's
`_meta["com.browseros.neo/run"].tasks`; the viewer reads them into `RunSummary.tasks`. Headers that belong to one HTTP hop (`connection`,
`transfer-encoding`, `content-length`, `host`) are set by each hop, as for any proxy. When the agent
goes away mid-reply the viewer closes its request to Neo, which is how Neo's stateless MCP learns to
stop the run.

## One list

Every call through the proxy is appended to one list, in the order it reached the viewer, whichever
agent sent it (owner, 2026-11-11: an agent that calls the MCP URL from a new curl or Python process
each time is still one job). Each run shows its client as a quiet label: its `clientInfo`, an older
client's `initialize`, or its User-Agent's product name, since several agents may share the list.
The tasks a run touched show beside it as quiet chips, one hue per task so a task's runs stand out
down the list: the ones its script created or picked (`neo.createTask`, `neo.task`, from the
reply's `_meta`), and the host's task (the request's `_meta["com.browseros.neo/run"].taskId`, which
picks the run's tab) marked "host". A closed row shows the first and "+N"; open, all of them. No
menu or filter: one list on purpose. A `taskId` tool argument is refused by Neo (`unknown field taskId`).
**Clear** (top bar, or `POST /api/clear`) empties it; the log before it is kept once, as
`log.prev.jsonl`, in case of a mis-click. Clear leaves Neo's tabs as they are: each run that names
no task leaves a tab, which Neo closes after 30 minutes idle. Stopping the viewer removes them only
with the container it started; with `--upstream` it started none, and Neo keeps its tabs.

## The log

On disk: one append-only JSONL file, `log.jsonl`, in the instance's state folder
(`--data-dir`, else `$NEO_VIEWER_DATA_DIR`, else `$XDG_STATE_HOME/neo-viewer/<instance>`, else
`~/.local/state/neo-viewer/<instance>`), beside `log.prev.jsonl` (the list before the last Clear),
the host token and `start-state.json`. The first
line starts the log; then one line per thing as it happened (`run-start` with the request body as
sent, `run-end` with the reply body as received, its text, structured content and `_meta`,
`neo-run` with Neo's record of the run, `tools-list` with the tool list a client read and its size).
A restart reads the file back, so nothing written is lost; a run that had started but not ended
when the viewer stopped shows as `lost`. Each start of `bun run neo:viewer` begins with an empty
log, as it does with a fresh browser profile, unless given `--keep-data` (or `--upstream`, which
starts no container). Tokens are estimates, characters divided by four, always shown with "≈".

## Export

**Export** (top bar, or `POST /api/export`) writes every run since the last Clear to a new folder
for an agent to read, and shows its path with Copy path until dismissed; the terminal prints it too:

```text
/tmp/neo-viewer/<instance>/export-<YYYYMMDD-HHMMSS>/
  README.md            start here: what this is, totals, one row per run, how to read the rest
  session.json         the same facts as data, each run's file paths; no code or replies inline
  runs/NNN.md          one run top to bottom (shared/run-markdown.ts): code as the page lays it
                       out, each line's methods, every operation, the reply's first 2,000 chars
  runs/NNN.json        that run complete: request and reply as they crossed, Neo's record
  runs/NNN.reply.txt   the text the model read, alone (.reply.json when it is JSON)
  log.jsonl            the raw log
```

Two more when Neo sent metrics (none for an image older than telemetry):

```text
  metrics/metrics.prom   Neo's last GET /v1/metrics answer, byte for byte (when the summary is the scrape's)
  metrics/summary.json   the metrics read out: health since the last Clear, runs, operations,
                         errors, tabs, crashes, ten minutes of history, every family
```

It lives outside the state folder, so it survives the viewer stopping and Clear. An agent can start
at one small file and open only what a question needs: README.md stays under about 300 lines however
many runs there are. No host token or Authorization header is ever in an export. Screenshots are
not exported.

**Copy** on a run row (at its right end: on hover, on keyboard focus, always on an open row) puts that
run's Markdown on the clipboard, from the same renderer as `runs/NNN.md` but with the whole reply:
paste it into any agent and ask about the run. A closed row's run is fetched at the click.

A returned string, such as a snapshot, reaches the model as one line of JSON (`\n`, `\"`). The plain
page, Copy and `runs/NNN.md` show it decoded, as lines, saying so and giving the JSON string's length;
the sizes and token counts stay those of what the model received, and the exact text stays in the
Raw view, `runs/NNN.json` and the reply file. The value is the reply's first line: Neo writes it as
one line of JSON, so every line after it (console output, Neo's notes, its `Tip:` lines, any line
it adds later) is shown under the value as the model read it, and never stops the value decoding.

## The API

`shared/api.ts` holds the routes and their types; the server owns them and the page imports them.

| Route | Answer |
|---|---|
| `GET /api/info` | `ViewerInfo`: the MCP URL, the upstream, the data folder, and the container the command started (`container: {name, image, up}`, null with `--upstream`) |
| `GET /api/runs` | `RunsResponse`: every `RunSummary` since the last Clear, in order, and the log's totals (`LogSummary`) |
| `GET /api/runs/:runId` | `RunResponse`: one `RunDetail`, request and reply as they crossed |
| `GET /api/traces` | Search retained run traces by partial `traceId`, exact `method`, and operation duration in milliseconds (`minMs` exclusive, `maxMs` inclusive). Pass `nextCursor` as `cursor` with the same filters for the next page. The default page size is 100 and `limit` accepts 1–200. |
| `GET /api/runs/:runId/trace` | `RunTraceResponse`: the run's spans for the waterfall (`trace: null` when none arrived) |
| `GET /api/metrics` | `MetricsResponse`: Neo's metrics from the live source (`metrics: null` when none is live) |
| `POST /api/clear` | `RunsResponse`, empty: starts a new log, keeps the previous as `log.prev.jsonl`; only from the viewer's own page |
| `POST /api/export` | `ExportResponse` (`dir`, `files`, `runs`): writes the folder above; only from the viewer's own page |
| `GET /api/events` | SSE: one `StreamEvent` per change: `{type:'run'}`, `{type:'log'}` (totals), `{type:'clear'}`, and `{type:'health'}` (no `version`; keep the highest `health.rev`) |
| `POST /otlp/v1/traces`, `/otlp/v1/metrics` | Neo's OpenTelemetry push (not the page's; see Neo's telemetry) |

The tabs one browser has open on one viewer share one `/api/events` stream. A browser opens at
most six HTTP/1.1 connections to one origin and a stream holds one for as long as its page is
open, so a stream per tab would leave six tabs no connection for anything else. The tab that holds
the `neo-viewer:events` Web Lock opens the stream and relays it to the other tabs over a
BroadcastChannel; when it closes, the next tab in line opens it and every tab takes a fresh
snapshot (`web/viewer-events.ts`). Not yet measured: an owning tab hidden long enough for Chrome to
throttle its timers (its relay runs on stream and channel events, not timers).

Once a run's reply is in, the viewer reads Neo's record of it through the host API
(`GET /v1/runs/{id}?wait=10`, joined by the reply's `_meta["com.browseros.neo/run"].id`, which is
`RunSummary.neoRunId`) and keeps it in the log. `RunDetail.neo` holds it: the run's status,
Neo time and error, and each operation's name, kind, effect, script site `{line, column}`, start
offset, duration, args, result, notes, delivery and error (`NeoRunRecord`, `NeoOperation` in
`shared/api.ts`). `RunSummary` gains `neoDurationMs` and `operationCount`. Neo bounds stored args
(4 KiB) and results (2 KiB); the full reply the model read is the proxy's `reply.text`.

## Neo's host API

`server/hostApi.ts` is the viewer's one client for Neo's host API (`/v1`: runs, operations, tabs,
the display, takeovers). `bun run neo:viewer` configures it with the container it starts; with
`--upstream`, pass `--host-token-file` (and `--host-api`, default `http://127.0.0.1:9300`).

- `hostFetch(path, init)`: a request on the host API, bearer token added; Neo's `Response` comes
  back for `.json()` or `.arrayBuffer()`. With no host API the viewer answers in Neo's error shape:
  `503 no_host_api`, or `502 host_api_unreachable` when nothing answers.
- `hostWebSocketTarget(path)`: `{ url, headers }` for an upgrade route such as
  `/v1/display?mode=view`, opened server-side with `new WebSocket(url, { headers })`. Neo accepts
  the token only as a header, so it never goes in a URL and never reaches the browser.

## Neo's telemetry

Neo emits OpenTelemetry (wire-contract: 16 metrics, a `neo.run` span per run and a span per
operation, named after the operation: `page.goto`, or `unknown` for a name Neo does not serve). The
viewer tells the two apart by `neo.span.type` (`run` or `op`), never by name, and reads both of
Neo's doors with no protobuf library (`server/telemetry/`, one module behind `startTelemetry`):

- **Push**: `POST /otlp/v1/traces` and `/otlp/v1/metrics`, OTLP/HTTP JSON, plain or gzip. The viewer
  stays on 127.0.0.1; its own container reaches it as `host.docker.internal`, because Docker Desktop
  carries a container's connection to that name to the host's loopback (probed on this machine,
  Docker Desktop 29.9.3: a container on its own network reached a server bound to 127.0.0.1, which
  saw `Host: host.docker.internal:<port>` from 127.0.0.1). That is the least exposed way: no second
  listener, nothing bound beyond loopback, no port published back. These routes are write-only and
  keep their own guard instead of the page's: `Host` a loopback name or `host.docker.internal`, no
  `Origin` (every browser POST has one, an exporter never does), `Content-Type: application/json`
  only (protobuf gets a 415 that names `OTEL_EXPORTER_OTLP_PROTOCOL=http/json`; a cross-site form
  cannot send JSON without a preflight the viewer never answers), at most 4 MiB after gunzip. On a
  Linux engine `host.docker.internal` is the bridge gateway, where a loopback viewer is not
  listening: pushes do not arrive there, and the scrape below still does.
- **Scrape**: `GET /v1/metrics` on the host API every 2 s. An image older than telemetry answers 404:
  the poller stops for good, silently.
- **Names**: the push sends OpenTelemetry names and keys (`neo.run.duration` with
  `neo.run.status`, `error.type` for an error code); the scrape serves the Prometheus names
  OpenTelemetry's compatibility rules derive from them (`neo_run_duration_seconds{neo_run_status}`).
  The viewer applies the same rules to a pushed metric (`otlp-names.ts`), as a collector or
  Prometheus' own OTLP receiver would, so the health bar, the Health view and an export read one
  set of names, and push alone (`--upstream`) shows the same numbers. A navigation's HTTP status
  arrives as the integer `http.response.status_code` and is filed as `http_response_status_code`.
  An operation's source line arrives as `code.line.number` and `code.column.number`.
- **Which numbers**: the scrape while it is at most 30 s old, else the push while it is, else none;
  never blended (both read one meter provider). Health (`TelemetryHealth`: runs in flight, tabs
  open, and counts since the last Clear with Neo restarts handled) goes to the page as an
  unversioned `health` event at most once a second, and in the runs snapshot.
- **Housekeeping**: Neo's operation metrics count the script's operations and the ones Neo runs for
  it. Its own housekeeping, sent with no source line (frame-tree reads, the event pump, listener
  bookkeeping), grows with open tabs, so Neo leaves it out of them; its spans still arrive and fold
  under **Neo internal**.
- **Which ops**: a run's op table is `RunDetail.neo`, from one source chosen at start. With a host
  API, Neo's `/v1/runs/{id}`, as before, and spans are only the run's trace. Without one
  (`--upstream`), the record is folded from the run's `neo.run` and operation spans
  (`source: 'spans'`; spans carry no args, results or notes), so the Calls column, method rows,
  Copy, export and a restart work the same. Spans are filed by Neo's run id (`neo.run.id`, the reply's `_meta` id),
  never by trace id, which a caller may reuse; a run with no reply (cancelled) is found by the span
  id the viewer minted for it.
- **Older images**: no spans, no metrics: everything above stays empty and the page is as before.

`bun server/telemetry/feed-fixture.ts serve` starts a stand-in Neo (MCP and host API) and
`… feed --viewer <url>` sends one run through a viewer and then, as Neo would, its spans (eight
operations, one failed) and the 16 metrics: telemetry to build the page against before Neo emits any.

## How the live screen works

The same panel docks in the designed screen's sidebar and sits beside the plain list. The browser
cannot reach Neo's host API (a bearer token, which a browser cannot put on a WebSocket), so the viewer
relays it and adds the token on its side; `shared/live-screen.ts` holds the routes and types.

```text
 page (noVNC) ──ws /api/live/display?mode=…──▶ viewer ──ws /v1/display?mode=… + token──▶ Neo
 page ──/api/live/status, /api/live/takeovers──▶ viewer ──/v1/tabs, /v1/trace, /v1/takeovers──▶ Neo
```

| On the panel | What it opens on Neo |
|---|---|
| Follow agent | `GET /v1/display?mode=view&followTab=<tab>`: watch; Neo keeps the agent's tab in front, the tab of the most recent run's last operation (from `/v1/trace`; subscriptions act on no tab, `newPage` counts with its new tab). When that run's request named a host task (`_meta…taskId`, see One list), `followTask=<task>` instead, so Neo follows it to a new tab by itself; a task a script picks with `neo.task` is not in Neo's trace, so such a run is followed by its tab; by tab, the panel moves when its status poll (1.5 s) sees the agent on another tab |
| Control | `GET /v1/display?mode=control`: clicks and keys reach the browser; the front tab stops following; the agent is not held |
| Take over / Give back | `POST /v1/takeovers {tabId, viewer}` for the agent's tab as the panel shows it, then `display?mode=control&release=<id>`; the agent's calls on that tab fail with `owner_has_control` until `POST /v1/takeovers/{id}/release`, or until the takeover expires (the viewer asks for no TTL, so Neo's default of 20 minutes applies). `viewer` is Neo's own screen size, so Neo's fit brings the tab to the front and leaves the screen as it is; Neo puts the window back on release |

The relay passes RFB frames and close codes through unchanged (Neo's `4002 released` ends a
takeover's display) and closes with its own codes for what Neo cannot say: 4901 the host API is not
reachable (no container), 4902 Neo refused the display (the reason carries Neo's error), 4903 the
connection dropped. The panel says so and reopens the display on its own, so it comes back when the
container restarts. The display and both POSTs take only requests whose `Origin` is the viewer's own
page: a WebSocket is not covered by the same-origin policy. The viewer gives back the takeovers it
holds when it stops.

Docked, the screen is as wide as the sidebar and as tall as Neo's screen ratio makes it (the
framebuffer size its RFB server reports; 1600×1000 until then), so dragging the sidebar wider zooms
in, live while dragging. It is never taller than the sidebar's visible height less the panel's
header row; only then does the picture letterbox. "What the model read" sits below it and the
sidebar scrolls (`web/live-screen/screen-fit.ts`).

The components are self-contained (`web/live-screen/`): `useLiveScreen()` makes the controller, and
`<LiveScreen size="docked" | "large">`, `<LiveScreenPopup>` and `<TakeOverButton>` take it. A page
makes one controller so the docked panel, the pop-up and Take over agree.

Neo's VNC server (TigerVNC's Xvnc) blocks a client address for 10 s, then twice as long each time,
after five connections that end before their handshake finishes, and every display reaches it from
Neo's own 127.0.0.1. So the panel opens a display only after it has stayed on screen for 150 ms; a
script that opens displays and drops them mid-handshake can still block every display for a while.

## Layout and checks

- `server/`: the Bun server. `main.ts` and `launch.ts` are the one command (`config.ts` its flags,
  `dependencies.ts` the package check, `neo-container.ts` Docker); `proxy/` forwards and reads MCP
  (`trace-context.ts` the `traceparent` rule), `log/` the JSONL log and the in-memory store,
  `neo-runs.ts` joins Neo's run records from one source, `hostApi.ts` is the host API client,
  `telemetry/` Neo's OpenTelemetry (only `index.ts` is imported from outside it; `fixtures.ts` and
  `feed-fixture.ts` play it), `export/` writes an export, `api/` the routes, `live-screen/` the
  display relay and takeover routes.
- `web/`: the page, React and Tailwind as in `apps/claw-app`: `web/screen/` the designed screen,
  `web/App.tsx` the plain list, `web/live-screen/` the live-screen panel. `server/page.ts` bundles it
  in memory when the viewer starts (`Bun.build`, about 40 ms), compiling Tailwind with the same
  `@tailwindcss/node` and `oxide` packages claw-app's Vite plugin uses.
- `shared/api.ts`: the API's types; `shared/live-screen.ts`: the live screen's;
  `shared/code-format.ts` (with `code-gaps.ts`, `code-tree.ts`): a script laid out by statement,
  whitespace only, with sites mapped to display lines; `shared/run-markdown.ts`: one run as Markdown,
  for an export's `runs/NNN.md` and a run row's Copy.

```sh
bun run test                     # in this folder: unit tests, the proxy and the relay against a fake Neo, the screen
NEO_VIEWER_REAL_UPSTREAM=http://127.0.0.1:9010/mcp bun run test:real-image
# and the live screen, with the host API on TCP (CLAW_HOST_TCP=0.0.0.0:9300, port published on 127.0.0.1):
NEO_VIEWER_REAL_UPSTREAM=… NEO_VIEWER_REAL_HOST=http://127.0.0.1:9300 \
  NEO_VIEWER_REAL_HOST_TOKEN_FILE=<token file> bun run test:real-image
```

The live-screen real-image test speaks RFB through the relay without a browser: it follows the agent,
clicks and types in control mode, takes over (the agent's run fails with `owner_has_control`) and
gives back (Neo closes the owner's display with 4002).

The proxy's real-image test puts a TCP tap between the viewer and the image and checks that each reply the
agent read is the bytes Neo sent, for a plain value, an error, console lines, a blob marker and a
streamed reply. Neo streams its 5 s keep-alive only to 2025-06-18 clients: a 2026-07-28 reply is one
event at the end, however long the run takes.
