# Migrating from hosted `/v1` (claw-server) to the claw-sdk API

When the Neo Docker image runs claw-sdk instead of claw-server, the hosted `/v1` API on port
9010 goes away. There is no compatibility shim, since zuzu is the only client. This note maps
every old call to its replacement in [`openapi.yaml`](openapi.yaml).

## What changes for every call

| | hosted `/v1` (claw-server) | claw-sdk API |
|---|---|---|
| Where | `http://127.0.0.1:9010/v1/…`, the same port as `/mcp` | the host listener: the Unix socket `CLAW_HOST_SOCKET` (default `$BROWSERCLAW_DIR/host.sock`), or `CLAW_HOST_TCP` when set. Never the MCP port |
| Auth | none | `Authorization: Bearer <token>`, with the token from the file `CLAW_HOST_TOKEN_FILE` names; on the socket, the peer uid too |
| Sessions | every call names a session | none. Neo issues no session id. Privacy and locks belong to a browser tab (`tabId`), and you group a task's runs with your own `taskId` |
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
| `GET /v1/sessions/{id}?include=screenshots` (zuzu's `Audit`) | `GET /v1/trace?taskId={taskId}&tail=20&screenshots=last:4` | Already masked and privacy-applied. `visuals: withheld_private_input` replaces zuzu's own check. `tail` and `screenshots=last:N` replace `trimNeoAudit`. Add `tab={tabId}` to keep only the operations on one tab |
| `GET /v1/sessions/{id}?wait=60` | `GET /v1/runs/{runId}?wait=60` | Completeness is per run (`complete`), not per session |
| `toolCalls[]` | `GET /v1/runs/{runId}`: `operations[]` | `parentId` links wrap retries, host calls and protected fills; `tabId` is the tab each operation targeted. The run id is on the `run` result's `_meta["com.browseros.neo/run"].id` |
| — (zuzu's before/after audit diff) | `GET /v1/runs?requestId={toolCallId}&taskId={taskId}` | With a `requestId`, a lookup after a transport timeout says whether the run started and what it sent. Neo always returns its own run id |
| `GET /v1/sessions/{id}/recording` (rrweb) | — | Recording is deferred. The old endpoint returned no pages in the Docker image anyway, because the bundled extensions are off |
| `GET /v1/sessions/{id}/screencast?quality=&tab=` | `GET /v1/tabs/{tabId}/screencast?quality=` | The tab is in the path; there is no stream that follows a session's current tab. Read parts by `Content-Length`, as today. The stream ends when the tab closes or becomes private, and `tab.closed` or `tab.private` on `/v1/events` replaces the five-second `Live()` poll |
| `GET /v1/sessions/{id}/video?quality=&tab=` | `GET /v1/tabs/{tabId}/video?quality=` | As for the screencast |
| `GET /v1/events?session=` | `GET /v1/events?taskId=&tab=` | `session.*` events are gone. `tab.private` and `tab.closed` are new, and run and operation events carry `taskId` and `tabId` |
| — (Xvnc on 127.0.0.1:5900, no auth) | `GET /v1/display?mode=view\|control&followTab={tabId}` or `&followTask={taskId}` | RFB over WebSocket. `view` drops input; `followTab` or `followTask` (not both) replaces zuzu's `follow`/`toFront`; `control` makes every open tab private before the first input |
| — (zuzu's handoff: `frontPage`, `fitOwnerView`, `giveBack`, `steerOwnerTab`) | `POST /v1/takeovers {tabId, viewer?, reads?, ttlSeconds?}` when the owner opens the link, `POST /v1/takeovers/{id}/release` when they are done | The flow composes small parts a host can also use alone: `POST /v1/locks {tabId, reads?, ttlSeconds}`, `POST /v1/tabs/{tabId}/private`, `POST /v1/tabs/{tabId}/fit` (+ `GET /v1/fits/{id}`, `…/navigate`, `…/restore`), `/v1/display`, `POST /v1/tabs/{tabId}/sweep`. Neo locks the agent out of the tab (`host / owner_has_control`) and fits the page to the owner's screen. `reads: block` is the default; send `allow` to let the agent keep reading the tab. To pause a whole task, stop sending its runs |
| — (zuzu's `markPrivate`) | `POST /v1/tabs/{tabId}/private`; secrets to mask go to `POST /host/v1/secrets` | The flag lasts until the tab closes, across Neo restarts. The agent's screenshots of the tab then fail with `host / page_private` |

## Error bodies

The body shape is unchanged: `{code, message, requestId}`. The codes are new
(`unauthorized`, `expired`, `withheld`, `tab_not_found`, `page_private`, `lock_released`,
`fit_restored`, `takeover_released`, `privacy_unavailable`, …). As before, clients must accept
codes they do not know.
