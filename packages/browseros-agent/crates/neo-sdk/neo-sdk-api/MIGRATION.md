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
- `taskId` is your label for the calls of one task. It replaces the session for grouping. You
  list a task's runs, trace and events by it, and the agent's `page` starts on the task's last
  tab. An unknown `taskId` is fine, and it never expires.
- `hostData` is stored with the run, masked, and Neo never reads it.

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
with the existing secret-set masks and size bounds. `Run.codeWithheld: unavailable` still marks
code that was never stored, for example after a restart.

Existing trace rows remain readable: `[redacted]` arguments stay as recorded, and an older
run whose code was withheld still has no code. No storage migration or reconstruction is done.
