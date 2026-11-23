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
 *   GET  /api/runs/:runId       RunResponse         one run in full; `?wait=neo` first waits (about 3 s at
 *                                                   most) for Neo's record of a run that just ended
 *   GET  /api/runs/:runId/trace RunTraceResponse    the run's OpenTelemetry spans, for the waterfall
 *   GET  /api/traces            TracesResponse      bounded search of recorded spans joined to current runs
 *   GET  /api/metrics           MetricsResponse     Neo's metrics from the active source (scrape or push)
 *   POST /api/clear             RunsResponse        starts an empty log (the previous is kept once)
 *   POST /api/export            ExportResponse      writes every run since the last Clear to a new folder
 *   GET  /api/events            text/event-stream of StreamEvent, one JSON object per `data:`
 *
 * Neo's own OpenTelemetry reaches the viewer at `POST /otlp/v1/traces` and `POST /otlp/v1/metrics`
 * (OTLP/HTTP JSON; `server/telemetry/`). Those are Neo's door, not the page's: nothing here types
 * them, and no OTLP or Prometheus wire shape crosses to the page, only the domain shapes below.
 */

/** Paths of the routes above, so neither side spells one by hand. */
export const apiRoutes = {
  info: '/api/info',
  runs: '/api/runs',
  run: (runId: string, options: { waitForNeo?: boolean } = {}) =>
    `/api/runs/${encodeURIComponent(runId)}${options.waitForNeo ? '?wait=neo' : ''}`,
  runTrace: (runId: string) => `/api/runs/${encodeURIComponent(runId)}/trace`,
  traces: '/api/traces',
  metrics: '/api/metrics',
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

/** A task a run's script created or picked (`neo.createTask`, `neo.task`), as Neo's reply names it. */
export interface RunTask {
  /** `hn-top-5-3f9a`: the name's slug and 4 hex, or `t-` and 4 hex. */
  id: string
  /** The name given to `neo.createTask`, when the reply carries one. */
  name: string | null
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
   * The request's `_meta["com.browseros.neo/run"].taskId`, when it named one: the host's task,
   * which picks the run's tab. The viewer never adds one: Neo sees each call exactly as it was
   * sent. The tasks the script itself created or picked (`neo.createTask`, `neo.task`) are `tasks`.
   */
  agentTaskId: string | null
  /**
   * The tasks the run's script created or picked, in Neo's order, from the reply's
   * `_meta["com.browseros.neo/run"].tasks` (#3452); empty when it touched none or has not ended.
   */
  tasks: RunTask[]
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
  /**
   * The W3C trace context the run went to Neo with (`server/proxy/trace-context.ts`): the
   * caller's `traceparent` (header or `_meta`), or one the viewer minted and added as a header.
   * Null when the call carried none the viewer could read and it added none (a JSON-RPC batch of
   * several calls, an invalid caller value), and for runs logged before the viewer kept it.
   */
  trace: RunTraceContext | null
}

/** The W3C trace context of one run (`traceparent` = `00-<traceId>-<spanId>-<flags>`). */
export interface RunTraceContext {
  /** 32 lowercase hex. Display and export only: a caller may reuse one trace for many runs. */
  traceId: string
  /** 16 lowercase hex: the parent span Neo's `neo.run` span hangs under. */
  spanId: string
  /**
   * The viewer minted it and added the header. Only then is `spanId` this run's alone, so a run
   * with no reply (cancelled, Neo's reply lost) can still find its `neo.run` span by it.
   */
  injected: boolean
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
  /**
   * Operation records Neo dropped for this run (`traceGaps`, 0 when none): what is missing cannot
   * be paired with what is there, so the viewer does not guess across a gap.
   */
  traceGaps: number
  /** `off` when Neo omitted values at the run's end. Each operation follows its own end's policy. */
  values?: string
  /**
   * Who made this record. Absent (every record before telemetry) or `host`: Neo's
   * `GET /v1/runs/{id}`. `spans`: folded from Neo's OpenTelemetry `neo.run` and operation spans, the
   * viewer's only record of a run when it has no host API (`--upstream`). Spans carry no
   * arguments, results, notes, kind, effect or screenshots, so those operations have none
   * (`args: null`, `kind: ''`, …): say so once rather than per field.
   */
  source?: 'host' | 'spans'
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
  /** `off` when Neo omitted this operation's values; absent when it recorded them. */
  values?: string
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
  /** Where a Neo pushes its OpenTelemetry to this viewer (`OTEL_EXPORTER_OTLP_ENDPOINT`). */
  otlp: OtlpInfo
}

/**
 * The viewer's OTLP/HTTP JSON receiver, as an `OTEL_EXPORTER_OTLP_ENDPOINT` base (Neo POSTs to
 * `<base>/v1/traces` and `<base>/v1/metrics`, with `OTEL_EXPORTER_OTLP_PROTOCOL=http/json`).
 */
export interface OtlpInfo {
  /** For a Neo on this machine: `http://127.0.0.1:<port>/otlp`. */
  endpoint: string
  /**
   * For a Neo in a Docker Desktop container: `http://host.docker.internal:<port>/otlp`. The
   * container the viewer starts gets this one.
   */
  containerEndpoint: string
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
  /**
   * Neo's health as of this snapshot (null: no telemetry yet). It is not a change of the list:
   * keep it only when its `rev` is above the one shown (see `HealthEvent`).
   */
  health: TelemetryHealth | null
}

export interface RunResponse {
  run: RunDetail
}

/**
 * One message on `/api/events`, with the change's number (see `RunsResponse`):
 * - `run`: a run was added or changed; it carries the run's new state. A run whose Neo record
 *   grew (Neo's host API answered, or more of its spans arrived) is a `run` change too.
 * - `log`: the log's totals changed.
 * - `clear`: the log was cleared; drop every run shown and start from `log`.
 */
export type ViewerEvent = { version: number } & (
  | { type: 'run'; run: RunSummary }
  | { type: 'log'; log: LogSummary }
  | { type: 'clear'; log: LogSummary }
)

/**
 * Neo's health changed (at most once a second). It carries no `version`: it is not a change of
 * the run list, so it never joins the replay of versioned events over a late snapshot. Keep the
 * health with the highest `rev`, from events and snapshots alike.
 */
export interface HealthEvent {
  type: 'health'
  health: TelemetryHealth
}

/** What `/api/events` carries: the run list's versioned events, and health. */
export type StreamEvent = ViewerEvent | HealthEvent

// ─── Neo's telemetry: health, metrics, a run's trace ────────────────────────────────────────

/**
 * Neo's health in one full state (never a patch), from its metrics (`GET /v1/metrics` scraped
 * through the host API, or pushed over OTLP) and its spans. The numbers come from one source at
 * a time, never blended: the scrape while it is at most 30 s old, else the push while it is, else
 * none (`source: null`, numbers null: Neo stopped answering).
 */
export interface TelemetryHealth {
  /** Rises with every metric sample, span batch or change of source. Keep the highest seen. */
  rev: number
  /** Epoch ms of the active source's latest sample; "N s ago" is the page's to work out. */
  observedAt: number | null
  source: 'scrape' | 'push' | null
  /** A `neo.run` or operation span has arrived since the viewer started: runs have traces. */
  spansSeen: boolean
  /** Live gauges: runs Neo is running now, tabs open now. */
  runsInFlight: number | null
  tabsOpen: number | null
  /**
   * Counts since the last Clear, the frame the run list and its totals use: measured from Neo's
   * counters as they stood at the Clear (or at the first sample after it), so a Neo restart (a
   * counter going down) counts on from zero. An error is a run or operation Neo attributed to
   * someone (`error_owner` other than `none`); `crashes` are renderer crashes.
   */
  sinceClear: {
    runs: number
    runErrors: number
    ops: number
    opErrors: number
    crashes: number
  } | null
}

/** `GET /api/metrics`. */
export interface MetricsResponse {
  /** Null while no source is live (no telemetry yet, an older image, or Neo stopped answering). */
  metrics: MetricsView | null
}

/**
 * Neo's metrics as the Health view shows them, from the active source alone (cumulative since
 * Neo started, unlike `TelemetryHealth.sinceClear`). Durations are seconds, as Neo records them.
 */
export interface MetricsView {
  source: 'scrape' | 'push'
  /** Epoch ms of the sample. */
  observedAt: number
  runs: {
    total: number
    inFlight: number
    /** `neo_runs_total` by `neo_run_status`. */
    byStatus: Record<string, number>
    duration: Quantiles
  }
  /** Per op name (`unknown` for names outside Neo's method table), most called first. */
  operations: Array<{
    op: string
    total: number
    errors: number
    duration: Quantiles
  }>
  /** `neo_errors_total` by owner and code, most first. */
  errors: Array<{ owner: string; code: string; count: number }>
  tabs: {
    openByOrigin: Record<string, number>
    openedByOrigin: Record<string, number>
    closedByReason: Record<string, number>
    lifetime: Quantiles
  }
  crashes: number
  /** `neo_navigations_total` and `neo_navigation_errors_total`. */
  navigations: {
    total: number
    /** By HTTP status (`http_response_status_code`); `none` for a navigation that read no status. */
    byStatus: Record<string, number>
    /** The ones the network refused, by Chromium's net error (`error_type`). */
    errors: Record<string, number>
  }
  /** `neo_captcha_solves_total`, `neo_captcha_reasons_total` and `neo_captcha_spend_total`. */
  captchas: {
    total: number
    /** Solves by the CAPTCHA found and how they ended, most first. */
    solves: Array<{ family: string; verdict: string; count: number }>
    byReason: Record<string, number>
    /** What providers charged, in US dollars, by provider. */
    spendUsd: Record<string, number>
  }
  /** Every metric family as received, unknown ones included: the "raw" view. */
  families: MetricFamily[]
}

/** A duration histogram summed over its label sets. */
export interface Quantiles {
  count: number
  /** Read off the buckets (see `p50Is`, `p95Is`); null with no observations. */
  p50Sec: number | null
  p95Sec: number | null
  /** How to read `p50Sec` and `p95Sec`: never as more than the data supports. */
  p50Is: QuantileReading
  p95Is: QuantileReading
  /** Cumulative counts at Neo's own bucket bounds (read from the data, never assumed); `le: null` is +Inf. */
  buckets: Array<{ le: number | null; count: number }>
}

/**
 * - `estimate`: interpolated inside a bucket that slower observations lie above (Prometheus'
 *   `histogram_quantile`), so never above the slowest one;
 * - `atMost`: the quantile falls in the bucket holding the slowest observations, where
 *   interpolation could pass them (a 62 s run in the 60–120 s bucket would read 81 s): the value
 *   is that bucket's upper bound, to be read "≤ 120 s";
 * - `over`: it falls past Neo's largest finite bound: the value is that bound, read "> 120 s".
 */
export type QuantileReading = 'estimate' | 'atMost' | 'over'

/**
 * One metric family as the scrape or the push delivered it, in one model for both. A counter's
 * name ends in `_total` either way; a histogram's `value` is its observation count.
 */
export interface MetricFamily {
  name: string
  kind: 'counter' | 'gauge' | 'histogram' | 'unknown'
  help: string | null
  series: Array<{
    labels: Record<string, string>
    /** The value, or a histogram's count; null for NaN and ±Inf, which JSON cannot carry. */
    value: number | null
    /** Histograms: the sum of observations (null when not finite). */
    sum?: number | null
    /** Histograms: cumulative bucket counts, ascending, `le: null` last (+Inf). */
    buckets?: Array<{ le: number | null; count: number }>
  }>
}

/** `GET /api/runs/:runId/trace`. */
export interface RunTraceResponse {
  /** Null when no span of the run has arrived (an older image, push off, or not yet). */
  trace: RunTrace | null
}

/** Filters over actual recorded operation spans; minMs is exclusive, maxMs inclusive. */
export interface TraceSearchQuery {
  /** Case-insensitive partial W3C trace id, at most 32 hex characters. */
  traceId: string
  /** Exact, case-sensitive canonical span name, also used by Metrics; empty means any method. */
  method: string
  minMs: number | null
  maxMs: number | null
  /** Number of run slices returned, 1–200. */
  limit: number
  /** Validated continuation boundary in immutable viewer arrival order. */
  cursor: Pick<TraceRunSummary, 'sentAt' | 'id'> | null
}

export type TraceRunSummary = Pick<
  RunSummary,
  'id' | 'seq' | 'sentAt' | 'status'
>

/** One viewer run's slice of a trace; several rows may share a caller's W3C trace id. */
export interface TraceSearchResult {
  run: TraceRunSummary
  traceId: string
  /** Epoch ms of the first retained span. */
  startedAt: number
  /** First retained span start through last end, in ms. */
  durationMs: number
  spanCount: number
  /** Failed operations, using the same outcome rule as the waterfall. */
  errorCount: number
  /** Earliest operation satisfying every method/time filter; null for trace-id-only browsing. */
  matchedSpan: Pick<
    TraceSpan,
    'spanId' | 'name' | 'label' | 'durationMs'
  > | null
}

/** Current viewer runs with retained spans only; Clear immediately removes their search rows. */
export interface TracesResponse {
  traces: TraceSearchResult[]
  /** More matching run slices exist than the requested limit. */
  truncated: boolean
  /** Opaque continuation token, preserving the current filters; null on the final page. */
  nextCursor: string | null
}

/** One run's spans, found by Neo's run id (`neo.run.id`), never by trace id alone. */
export interface RunTrace {
  traceId: string
  /** The `neo.run` span has arrived: the trace is whole and will not change. */
  complete: boolean
  /** From the first span's start to the last one's end. */
  durationMs: number
  /** Depth first, siblings by start: the order rows are drawn. */
  spans: TraceSpan[]
  /** The sender's resource attributes (`service.name`, …), shown once. */
  resource: Record<string, string | number | boolean>
}

export interface TraceSpan {
  spanId: string
  parentSpanId: string | null
  /** `neo.span.type`: the run's own span, or one of its operations. */
  type: 'run' | 'op'
  /** The span's name as Neo sent it: `neo.run`, or an operation's (`page.goto`, `unknown`). */
  name: string
  /** An operation's `neo.op.name` (what the script called), or the `neo.run`'s run id. */
  label: string
  /** `neo.op.id` (= `NeoOperation.id`): links the span to its method row; null on `neo.run`. */
  opId: string | null
  /** Ms from the trace's first span start. */
  startOffsetMs: number
  durationMs: number
  /** 0 for `neo.run`, 1 for its operations, deeper for an operation inside another. */
  depth: number
  /**
   * An operation whose `neo.op.outcome` is `error`: what a trace's "failed" counts. A cancelled op
   * is not a failure, whatever its OTel status says, and `neo.run` is never one:
   * the run's own status is the run's.
   */
  error: boolean
  /** Every attribute as Neo emitted it: what a span carries is how redaction is checked. */
  attributes: Record<string, string | number | boolean>
}
