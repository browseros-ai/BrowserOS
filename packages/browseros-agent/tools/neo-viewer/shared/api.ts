/**
 * The neo viewer's HTTP API: the types the server answers with and the web UI reads.
 *
 * The server owns these types; the UI imports them and never redefines one. Everything here
 * comes from the viewer's own proxy log (what the agent sent and what Neo answered, as they
 * crossed the proxy), one list of every run in arrival order since the last Clear, joined to
 * Neo's own record of each run (`RunDetail.neo`).
 *
 * Routes (JSON unless noted):
 *   GET  /api/info              ViewerInfo
 *   GET  /api/runs              RunsResponse        every run since the last Clear, in order
 *   GET  /api/runs/:runId       RunResponse         one run in full
 *   POST /api/clear             RunsResponse        starts an empty log (the previous is kept once)
 *   POST /api/export            ExportResponse      writes every run since the last Clear to a new folder
 *   GET  /api/events            text/event-stream of ViewerEvent, one JSON object per `data:`
 */

/** Paths of the routes above, so neither side spells one by hand. */
export const apiRoutes = {
  info: '/api/info',
  runs: '/api/runs',
  run: (runId: string) => `/api/runs/${encodeURIComponent(runId)}`,
  clear: '/api/clear',
  export: '/api/export',
  events: '/api/events',
} as const

/** `POST /api/export`: the folder written, and its files relative to it (`README.md` first). */
export interface ExportResponse {
  /** Absolute: `/tmp/neo-viewer/<instance>/export-<YYYYMMDD-HHMMSS>`. */
  dir: string
  files: string[]
  runs: number
}

/** Tokens are estimates: characters divided by four, always shown with "≈". */
export interface TextSize {
  chars: number
  tokens: number
}

/** The one token estimate both sides use: characters (UTF-16 units) ÷ 4, rounded down. */
export function textSize(text: string): TextSize {
  return { chars: text.length, tokens: Math.floor(text.length / 4) }
}

/** What the agent said it is: `clientInfo` from `initialize` or a request's `_meta`. */
export interface ClientInfo {
  name: string
  version?: string
  title?: string
}

/** Every run since the last Clear, summed. */
export interface LogTotals {
  runs: number
  running: number
  /** Runs whose status is not `ok` or `running`. */
  failed: number
  /** Sum of the runs' round trips. */
  roundTripMs: number
  /** Sum of the gaps before each run (time between a reply and the next call). */
  gapMs: number
  /** From the first run sent to the last reply. */
  wallClockMs: number
  /** What the model wrote: the `code` of every run. */
  code: TextSize
  /** What the model read: the reply text of every run. */
  reply: TextSize
}

/** The log as a whole: its totals, and when it began. */
export interface LogSummary {
  /** ISO 8601: when the log began (the viewer's first start, or the last Clear). */
  clearedAt: string
  totals: LogTotals
  /** The latest `tools/list` reply's text as a client read it: the run tool's description and schema. */
  toolsList: TextSize | null
}

/**
 * - `ok`: a tool result without `isError`.
 * - `error`: a tool result with `isError` (a script, website or Neo failure the model reads).
 * - `rpc-error`: a JSON-RPC error instead of a result.
 * - `http-error`: Neo answered with a non-2xx status or a body without the reply.
 * - `cancelled`: the agent went away before the reply; the viewer closed its upstream request,
 *   which is how Neo learns to stop the run.
 * - `lost`: the viewer stopped before the reply arrived (found when the log is read back).
 */
export type RunStatus =
  | 'running'
  | 'ok'
  | 'error'
  | 'rpc-error'
  | 'http-error'
  | 'cancelled'
  | 'lost'

/** One `tools/call` through the proxy, as listed. */
export interface RunSummary {
  id: string
  /** 1-based position in the log, in the order the calls arrived. */
  seq: number
  /** The agent that sent it (its `clientInfo`, or its User-Agent's product); several may share the log. */
  client: ClientInfo | null
  /**
   * The agent's own `_meta["com.browseros.neo/run"].taskId`, when it sent one. The viewer never
   * adds one: Neo sees each call exactly as the agent sent it.
   */
  agentTaskId: string | null
  /** The tool called; Neo serves one, `run`. */
  tool: string
  status: RunStatus
  /** The script as the agent sent it (`arguments.code`). */
  code: string
  timeoutMs: number | null
  /** ISO 8601: when the call reached the viewer. */
  sentAt: string
  /** ISO 8601: when the reply passed back through; null while running. */
  answeredAt: string | null
  /** `answeredAt − sentAt`, what the agent waited. */
  roundTripMs: number | null
  /** Time the viewer held the call before forwarding it (finding the calling process). */
  proxyMs: number
  /** Since the previous run's reply; null for the first run or an overlapping one. */
  gapMs: number | null
  codeSize: TextSize
  replySize: TextSize | null
  /** The first characters of the reply text. */
  replyPreview: string | null
  /** `_meta["com.browseros.neo/run"].id` of the reply: the key for `GET /v1/runs/{id}` (joined later). */
  neoRunId: string | null
  /** `_meta["com.browseros.neo/run"].status` and `.effects`, as Neo reported them. */
  neoStatus: string | null
  neoEffects: string | null
  /** Time Neo spent on the run (its record's `endedAt − startedAt`); null until the record is joined. */
  neoDurationMs: number | null
  /** How many operations Neo recorded; null until the record is joined. */
  operationCount: number | null
}

/**
 * Neo's own record of a run, read from its host API (`GET /v1/runs/{id}`, openapi.yaml `Run`)
 * once the reply is in, and joined to the proxy's record by `neoRunId`. Neo stores operation
 * args at most 4 KiB and results 2 KiB; the reply the model read is the proxy's `reply.text`.
 */
export interface NeoRunRecord {
  /** `running`, `ok`, `error`, `cancelled`, `timeout`, `interrupted` (extensible). */
  status: string
  effects: string | null
  /** Epoch ms. */
  startedAt: number
  endedAt: number | null
  /** `endedAt − startedAt`: the time Neo spent on the run. */
  durationMs: number | null
  /** False when some operation had not ended when the record was read. */
  complete: boolean
  /** The tabs its operations targeted. */
  tabs: number[]
  error: NeoOperationError | null
  /** In start order. */
  operations: NeoOperation[]
}

/** One SDK call the script made (or one made inside it), as Neo recorded it (openapi.yaml `Operation`). */
export interface NeoOperation {
  /** `<runId>.<seq>`. */
  id: string
  /** Its place in the run, from 1, in start order. */
  seq: number
  /** The operation this one ran inside; null for the script's own calls. */
  parentId: string | null
  /** e.g. `page.goto`, `locator.click`. */
  name: string
  /** `builtin`, `defined`, `next`, `host_call`, `protected` (extensible). */
  kind: string
  /** `read` or `write`. */
  effect: string
  /** The script line and column that called it (1-based); null for nested calls and old records. */
  site: { line: number; column: number } | null
  /** Ms from the run's start to this operation's start. */
  startOffsetMs: number
  durationMs: number | null
  tabId: number | null
  /** Bounded by Neo; null while running. */
  args: unknown
  argsTruncated: boolean
  result: unknown
  resultTruncated: boolean
  /** `ok`, `error`, `cancelled`; null while running. */
  outcome: string | null
  /** `not_sent`, `done`, `unknown`; null while running. */
  delivery: string | null
  error: NeoOperationError | null
  /** Its call-log lines, oldest first. */
  notes: string[]
  hasScreenshot: boolean
}

/** openapi.yaml `OperationError`. */
export interface NeoOperationError {
  /** Whose behavior must change: `neo`, `script`, `website`, `host`. */
  owner: string
  code: string
  message: string
  retryable: boolean
  hostCode?: string
  operationId?: string
}

export interface JsonRpcError {
  code: number
  message: string
  data?: unknown
}

/** One run in full: the request and reply exactly as they crossed the proxy, and what was read from them. */
export interface RunDetail extends RunSummary {
  /** Neo's record of the run, joined by `neoRunId`; null until read, or when there is no host API. */
  neo: NeoRunRecord | null
  request: {
    /** The HTTP body the agent sent, byte for byte (UTF-8). */
    raw: string
    /** The JSON-RPC id of this call. */
    jsonRpcId: string | number | null
  }
  reply: {
    httpStatus: number | null
    contentType: string | null
    /** Neo's `Content-Encoding`, if any; the agent got the encoded bytes, `raw` holds them decoded. */
    contentEncoding: string | null
    /** The HTTP body Neo answered with, byte for byte (UTF-8): SSE framing and keep-alives included. */
    raw: string | null
    /** The text content blocks joined with "\n": what the model read. */
    text: string | null
    structuredContent: unknown
    isError: boolean | null
    /** The result's `_meta`, including `com.browseros.neo/run`. */
    meta: unknown
    rpcError: JsonRpcError | null
    /** Server messages streamed before the reply (progress, logging), in order. */
    notifications: unknown[]
    /** SSE comment lines (Neo's keep-alive every 5 s) seen while waiting. */
    keepAlives: number
  }
}

export interface ViewerInfo {
  /** The URL to give an agent: the viewer's MCP endpoint. */
  mcpUrl: string
  /** Where the viewer forwards MCP: the Neo SDK image's `/mcp`. */
  upstreamUrl: string
  /** The viewer's state folder, which holds the run log (`log.jsonl`). */
  dataDir: string
  startedAt: string
  /**
   * The Neo SDK container `bun run neo:viewer` started (read-only: the command is the control).
   * Null when the viewer forwards to an `--upstream` it did not start.
   */
  container: ContainerInfo | null
}

export interface ContainerInfo {
  name: string
  /** The image tag in use, e.g. `neo-sdk:0.52.1.2-d6b008abb-arm64`. */
  image: string
  /** Whether the container is running now. */
  up: boolean
}

/**
 * Snapshots carry `version`, the number of the last change they include; each event carries its
 * own. A reader that fetches a snapshot while the event stream runs applies the snapshot, then
 * only the events numbered above it.
 */
/**
 * `version` is the number of the last change the snapshot includes; each event carries its own.
 * A reader that fetches a snapshot while the event stream runs applies the snapshot, then only the
 * events numbered above it.
 */
export interface RunsResponse {
  log: LogSummary
  runs: RunSummary[]
  version: number
}

export interface RunResponse {
  run: RunDetail
}

/**
 * One message on `/api/events`, with the change's number (see `RunsResponse`):
 * - `run`: a run was added or changed; it carries the run's new state.
 * - `log`: the log's totals changed.
 * - `clear`: the log was cleared; drop every run shown and start from `log`.
 */
export type ViewerEvent = { version: number } & (
  | { type: 'run'; run: RunSummary }
  | { type: 'log'; log: LogSummary }
  | { type: 'clear'; log: LogSummary }
)
