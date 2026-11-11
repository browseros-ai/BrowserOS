/**
 * The neo viewer's HTTP API: the types the server answers with and the web UI reads.
 *
 * The server owns these types; the UI imports them and never redefines one. Everything here
 * comes from the viewer's own proxy log (what the agent sent and what Neo answered, as they
 * crossed the proxy). Fields marked "joined later" are left for the PRs that add Neo's host
 * API (operations, tabs, display): they extend these shapes rather than replace them.
 *
 * Routes (all GET, JSON unless noted):
 *   /api/info                    ViewerInfo
 *   /api/tasks                   TasksResponse       newest task first
 *   /api/tasks/:taskId/runs      TaskRunsResponse    the task and its runs, in the order sent
 *   /api/runs/:runId             RunResponse         one run in full
 *   /api/events                  text/event-stream of ViewerEvent, one JSON object per `data:`
 */

/** Paths of the routes above, so neither side spells one by hand. */
export const apiRoutes = {
  info: '/api/info',
  tasks: '/api/tasks',
  taskRuns: (taskId: string) => `/api/tasks/${encodeURIComponent(taskId)}/runs`,
  run: (runId: string) => `/api/runs/${encodeURIComponent(runId)}`,
  events: '/api/events',
} as const

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

/**
 * How the viewer decided which task a call belongs to. Neo's `/mcp` is stateless and current
 * clients keep no MCP session, so the viewer applies three rules in order
 * (`server/tasks/task-rule.ts`):
 * - `agent-task-id`: the call carried `_meta["com.browseros.neo/run"].taskId`; the agent named
 *   its task (zuzu does this).
 * - `process`: the calling process, found from the TCP connection; one task per process and
 *   client name, so two agents at once are two tasks.
 * - `client-idle`: the process could not be found; one task per client name, and a new one after
 *   `idleMinutes` without a call from that client.
 */
export type TaskIdentification =
  | { kind: 'agent-task-id'; taskId: string }
  | {
      kind: 'process'
      pid: number
      /** The process's start time as `ps` prints it, so a reused pid is a new task. */
      processStartedAt: string | null
      /** The executable as `ps` names it. */
      command: string | null
    }
  | {
      kind: 'client-idle'
      idleMinutes: number
      /** Why the calling process was not found. */
      reason: string
    }

/** One agent task: a group of `run` calls, named `<client>, HH:MM`. */
export interface TaskSummary {
  /** The viewer's id for the task; the URL and log-file key. */
  id: string
  /** `<client>, HH:MM`, from the client name and the task's first call (local time). */
  name: string
  /**
   * The `taskId` Neo saw on this task's runs: the agent's own for `agent-task-id`, otherwise the
   * viewer stamped `id` into `_meta["com.browseros.neo/run"].taskId`, so each task has its own
   * tab in Neo and `GET /v1/runs?taskId=` finds its runs.
   */
  neoTaskId: string
  client: ClientInfo
  identifiedBy: TaskIdentification
  /** ISO 8601. */
  createdAt: string
  lastCallAt: string
  /** The `tools/list` reply's text as the client read it, once per task (the first one seen). */
  toolsList: TextSize | null
  totals: TaskTotals
}

export interface TaskTotals {
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
  taskId: string
  /** 1-based position in the task, in the order the calls arrived. */
  seq: number
  /** The tool called; Neo serves one, `run`. */
  tool: string
  status: RunStatus
  /** The script as the agent sent it (`arguments.code`). */
  code: string
  timeoutMs: number | null
  /** True when the viewer added `taskId` to the call's `_meta` on its way to Neo. */
  taskIdStamped: boolean
  /** ISO 8601: when the call reached the viewer. */
  sentAt: string
  /** ISO 8601: when the reply passed back through; null while running. */
  answeredAt: string | null
  /** `answeredAt − sentAt`, what the agent waited. */
  roundTripMs: number | null
  /** Time the viewer held the call before forwarding it (finding the calling process). */
  proxyMs: number
  /** Since the previous run's reply in this task; null for the first run or an overlapping one. */
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
    /** The body forwarded to Neo when the viewer stamped a `taskId`; null when forwarded unchanged. */
    forwarded: string | null
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
  /** Where each task's JSONL log lives. */
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
export interface TasksResponse {
  tasks: TaskSummary[]
  version: number
}

export interface TaskRunsResponse {
  task: TaskSummary
  runs: RunSummary[]
  version: number
}

export interface RunResponse {
  run: RunDetail
}

/**
 * One message on `/api/events`: a task or run was created or changed; it carries the new state
 * and the change's number (see `TasksResponse`).
 */
export type ViewerEvent = { version: number } & (
  | { type: 'task'; task: TaskSummary }
  | { type: 'run'; run: RunSummary }
)
