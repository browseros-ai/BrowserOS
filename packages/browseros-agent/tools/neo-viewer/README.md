# Neo viewer

A local tool for one person: give an agent one MCP URL, and see every `run()` call it makes against
the Neo SDK image. For each call it shows the code as sent, the reply as the model read it
(formatted and raw), the round trip, the time between calls, and token estimates. It is a dev tool
under `tools/`: not shipped, and not part of the image.

```text
 agent ──MCP──▶ viewer (Bun, 127.0.0.1:7310/mcp) ──MCP──▶ Neo SDK image (127.0.0.1:9010/mcp)
                  │ records every call and reply, as sent and as received
                  ▼
               ~/.local/state/neo-viewer/tasks/<task>.jsonl ──▶ /api/* ──▶ the page at /
```

This is the viewer's first part (PR B): the proxy, the per-task log, the API and a plain run list.
Starting and stopping Docker, Neo's host API (operations, tabs), the live screen and the designed
Viewer C v2 screen come in the next PRs, on the same API.

## Start it

From `packages/browseros-agent`:

```sh
bun run neo:viewer
```

That is the whole loop. It works out this checkout's Neo SDK image the way `build.sh` names it
(`neo-sdk:<browser>-<commit>[-dirty]-<arch>`), builds it with `build.sh` when it is not on the
machine or the checkout's uncommitted build inputs changed since the viewer built it, starts it
fresh, and prints what follows. A first build takes about 7 minutes; a new commit that leaves the
server alone rebuilds from Docker's cache in seconds, and uncommitted changes outside the image's
build inputs (this tool, say) reuse the commit's image:

```text
  Viewer:         http://127.0.0.1:7310
  Agent MCP URL:  http://127.0.0.1:7310/mcp
  Claude Code:    claude mcp add --transport http neo-viewer http://127.0.0.1:7310/mcp
```

Ctrl-C stops the viewer and removes the container and network it started. Each start begins
clean: it removes what the previous start left (the container, network and data) and, after a new
build, the image the previous build made.

| Flag | |
|---|---|
| `--keep-data` | keep the browser profile (logins) and the run history between starts |
| `--no-build` | use the newest `neo-sdk` image already on this machine |
| `--image <tag>` | use this image |
| `--upstream <url>` | forward to a Neo MCP endpoint you started yourself; no container |
| `--port <port>` | the viewer's port (7310) |
| `--data-dir <dir>` | the viewer's state (below) |

Pass flags after `--`: `bun run neo:viewer -- --keep-data`. The same settings come from
`NEO_VIEWER_PORT`, `NEO_VIEWER_UPSTREAM` and `NEO_VIEWER_DATA_DIR`. The viewer listens on 127.0.0.1
only.

The container is started as [the image's README](../../crates/neo-sdk/neo-server/docker/README.md)
shows for a standalone start: its own Docker network, `NEO_MCP_REMOTE=true`, and the host API over
the development option `CLAW_HOST_TCP` with a token file the command writes (mode 0600) in the
viewer's state folder; both ports are published on 127.0.0.1 at ports Docker picks. Inside the
container `CLAW_HOST_TCP` binds `0.0.0.0:9300`: bound to `127.0.0.1` the published port answers
nothing, since Docker's port forward arrives from outside the container's loopback.

It touches only what it created: the container `neo-viewer-sdk`, the network `neo-viewer-sdk-net`
and the volume `neo-viewer-sdk-data`, each carrying the label `com.browseros.neo-viewer`, and the
image its own last build recorded in `start-state.json`. A same-named thing without the label
stops the start instead. `GET /api/info` reports the image in use and whether the container is up;
the command is the only control.

## What the proxy changes

Nothing, except one field. Every method, path, header and body reaches Neo as the agent sent it,
and every reply reaches the agent as Neo sent it, byte for byte, chunk by chunk as it streams. A
`run` call that has no `_meta["com.browseros.neo/run"].taskId` gets the viewer's task id added
there, so each task has its own tab in Neo and `GET /v1/runs?taskId=` finds its runs. The stamp is
spliced into the text as sent; nothing else in the body is re-serialized. Headers that belong to
one HTTP hop (`connection`, `transfer-encoding`, `content-length`, `host`) are set by each hop, as
for any proxy. When the agent goes away mid-reply the viewer closes its request to Neo, which is
how Neo's stateless MCP learns to stop the run.

## Which task a call belongs to

MCP would make each agent connection a session, but there is none to use: Neo's `/mcp` is stateless
(it never sends `Mcp-Session-Id`), and current clients keep no session either. Claude Code 2.1
speaks the 2026-07-28 revision: no `initialize`, `clientInfo` in every request's `_meta`, nothing
per session, and it ignores a session id a server offers. So the viewer decides, in this order
(`server/tasks/task-rule.ts`, the one module to replace to change it):

1. **The agent's task id.** The call carries `_meta["com.browseros.neo/run"].taskId` (zuzu does):
   that is the task.
2. **The calling process.** The viewer finds the process at the other end of the TCP connection
   (`netstat -anv` on macOS, `ss -tnp` on Linux, then `ps` for its start time, so a reused pid is a
   new process). One task per process and client name: two Claude Codes are two tasks.
3. **The client name, with an idle gap.** When the process cannot be found (the client closed its
   connection before the lookup finished, or no socket table reader), one task per client name, and
   a new one after 10 minutes without a call from that client.

Each task records which rule grouped it (`identifiedBy` in `shared/api.ts`), and is named
`<client>, HH:MM` from the client's name and its first call. A client that never sends `clientInfo`
is named from its `initialize` (remembered by connection, process and User-Agent), then from its
User-Agent. Calls from a container arrive through Docker Desktop's forwarder, so they share one
process (`com.docker.backend`) and are grouped by rule 1 or by client name.

The stamp has to name the task before the call goes to Neo, so a `run` without a task id waits for
the process lookup: about 30 to 60 ms on the first call of a connection, cached while the
connection is in use, and never more than 500 ms (then rule 3). Every run records that wait as
`proxyMs`; `roundTripMs` is what the agent waited, from the call reaching the viewer to its reply
leaving it. Calls that name their task, and every other request, are forwarded at once.

## The log

Append-only JSONL, one file per task, under `$NEO_VIEWER_DATA_DIR`, else
`$XDG_STATE_HOME/neo-viewer`, else `~/.local/state/neo-viewer`: `tasks/<task id>.jsonl`. The first
line describes the task; then one line per thing as it happened (`run-start` with the request body
as sent and as forwarded, `run-end` with the reply body as received, its text, structured content
and `_meta`, `tools-list` with the size of the tool list the client read). A restart reads every
file back, so nothing written is lost; a run that had started but not ended when the viewer stopped
shows as `lost`. `bun run neo:viewer` clears the history at each start, as it does the browser
profile, unless given `--keep-data` (or `--upstream`, which starts no container). Tokens are estimates, characters divided by four, always shown with "≈".

## The API

`shared/api.ts` holds the routes and their types; the server owns them and the page imports them.

| Route | Answer |
|---|---|
| `GET /api/info` | `ViewerInfo`: the MCP URL, the upstream, the data folder, and the container the command started (`container: {name, image, up}`, null with `--upstream`) |
| `GET /api/tasks` | `TasksResponse`: every `TaskSummary`, newest first, with totals |
| `GET /api/tasks/:taskId/runs` | `TaskRunsResponse`: the task and its `RunSummary` list, in order |
| `GET /api/runs/:runId` | `RunResponse`: one `RunDetail`, request and reply as they crossed |
| `GET /api/events` | SSE: one `ViewerEvent` (`{type:'task'}` or `{type:'run'}`) per change |

Once a run's reply is in, the viewer reads Neo's record of it through the host API
(`GET /v1/runs/{id}?wait=10`, joined by the reply's `_meta["com.browseros.neo/run"].id`, which is
`RunSummary.neoRunId`) and keeps it in the task log. `RunDetail.neo` holds it: the run's status,
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

## Layout and checks

- `server/`: the Bun server. `proxy/` forwards and reads MCP, `tasks/` holds the task rule and the
  process lookup, `log/` the JSONL log and the in-memory store, `api/` the routes.
- `web/`: the page, React and Tailwind as in `apps/claw-app`. `server/page.ts` bundles it in memory
  when the viewer starts (`Bun.build`, about 40 ms), compiling Tailwind with the same
  `@tailwindcss/node` and `oxide` packages claw-app's Vite plugin uses.
- `shared/api.ts`: the API's types.

```sh
bun run test                     # in this folder: unit tests and the proxy against a fake Neo
NEO_VIEWER_REAL_UPSTREAM=http://127.0.0.1:9010/mcp bun run test:real-image
```

The real-image test puts a TCP tap between the viewer and the image and checks that each reply the
agent read is the bytes Neo sent, for a plain value, an error, console lines, a blob marker and a
streamed reply. Neo streams its 5 s keep-alive only to 2025-06-18 clients: a 2026-07-28 reply is one
event at the end, however long the run takes.
