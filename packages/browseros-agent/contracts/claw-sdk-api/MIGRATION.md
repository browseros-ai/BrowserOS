# Migrating from hosted `/v1` (claw-server) to the claw-sdk API

When the Neo Docker image runs claw-sdk instead of claw-server, the hosted `/v1` API on port
9010 goes away. There is no compatibility shim, since zuzu is the only client. This note maps
every old call to its replacement in [`openapi.yaml`](openapi.yaml).

## What changes for every call

| | hosted `/v1` (claw-server) | claw-sdk API |
|---|---|---|
| Where | `http://127.0.0.1:9010/v1/…`, the same port as `/mcp` | the host listener: the Unix socket `[host_api] socket`, or `[host_api] tcp` when set. Never the MCP port |
| Auth | none | `Authorization: Bearer <token>` from `[host_api] token_file`; on the socket, the peer uid too |
| Unit of work | a session's tool calls (`toolCalls[]`) | a session's runs, each with an operation tree (`runs[]`, `operations[]`) |
| Failure | `outcome: error` | `outcome` plus `delivery` (`not_sent` / `done` / `unknown`) and `error.owner` (`neo` / `script` / `website` / `host`) |

Go dials the socket with a custom `DialContext` on its `http.Transport`. The URL host is then
ignored, so `http://neo/v1/…` works.

## Call by call

| hosted `/v1` | claw-sdk API | Notes |
|---|---|---|
| `GET /v1/sessions?client=&since=&until=&q=&cursor=&limit=` | `GET /v1/sessions?live=&since=&until=&cursor=&limit=` | `q` and `client` filters are gone; `name`, `site` and `summary` are not recorded |
| `GET /v1/sessions/{id}` (`status`, `toolCalls[].page`) | `GET /v1/sessions/{id}` | `status` is `live` / `ended`. `currentPage` replaces scanning `toolCalls` for the newest `page.tabId` (zuzu's `CurrentTab`) |
| `GET /v1/sessions/{id}?include=screenshots` (zuzu's `Audit`) | `GET /v1/sessions/{id}/trace?tail=20&screenshots=last:4` | Already masked and privacy-applied. `visuals: withheld_private_input` replaces zuzu's own check. `tail` and `screenshots=last:N` replace `trimNeoAudit` |
| `GET /v1/sessions/{id}?wait=60` | `GET /v1/runs/{runId}?wait=60` | Completeness is per run (`complete`), not per session |
| `toolCalls[]` | `GET /v1/runs/{runId}`: `operations[]` | `parentId` links wrap retries, host calls and protected fills. The run id is on the `run` result's `_meta["com.browseros.neo/run"].id` |
| — (zuzu's before/after audit diff) | `GET /v1/runs?requestId={toolCallId}` | Send `_meta["com.browseros.neo/run"] = {requestId?, hostData?}` with `run`; both are optional. Neo always returns its own run id. With a `requestId`, a lookup after a transport timeout says whether the run started and what it sent, and a repeat in the same session never runs again |
| `GET /v1/sessions/{id}/recording` (rrweb) | — | Recording is deferred. The old endpoint returned no pages in the Docker image anyway, because the bundled extensions are off |
| `GET /v1/sessions/{id}/screencast?quality=&tab=` | the same path and parameters on the host listener | Read parts by `Content-Length`, as today. The stream ends with the session, its pinned tab or privacy, and `session.ended` on `/v1/events` replaces the five-second `Live()` poll |
| `GET /v1/sessions/{id}/video?quality=&tab=` | the same path and parameters on the host listener | — |
| — (Xvnc on 127.0.0.1:5900, no auth) | `GET /v1/display?mode=view\|control&follow={session}` | RFB over WebSocket. `view` drops input; `follow` replaces zuzu's `follow`/`toFront`; `control` applies privacy |
| — (zuzu's handoff: `frontPage`, `fitOwnerView`, `giveBack`, `steerOwnerTab`) | `POST /v1/takeovers` when the owner opens the link, `POST /v1/takeovers/{id}/release` when they are done | The flow composes small parts a host can also use alone: `POST /v1/locks`, `POST /v1/sessions/{id}/private`, `POST /v1/tabs/{tabId}/fit` (+ `GET /v1/fits/{id}`, `…/navigate`, `…/restore`), `/v1/display`, `POST /v1/tabs/{tabId}/sweep`. Neo locks the agent out (`host / owner_has_control`) and fits the page to the owner's screen |
| — (zuzu's `markPrivate`) | `POST /v1/sessions/{id}/private`; secrets to mask go to `POST /host/v1/secrets` | — |

## Error bodies

The body shape is unchanged: `{code, message, requestId}`. The codes are new
(`unauthorized`, `expired`, `withheld`, `session_private`, `lock_released`, `fit_restored`, `takeover_released`,
`privacy_unavailable`, …). As before, clients must accept codes they do not know.
