/**
 * Joins Neo's own record of each run to the proxy's record.
 *
 *   proxy: a run's reply passes ─▶ store emits the ended run, with `neoRunId` from the reply's _meta
 *   joiner: source.read({neoRunId, trace}) ─▶ NeoRunRecord ─▶ store.attachNeoRun
 *
 * The proxy has what the agent sent and read; only Neo has the operations (each call's name,
 * site, timing, outcome, error). Where the record comes from is chosen once, when the viewer
 * starts, and never changes, so a run's op table never switches source or shows an op twice:
 * - a host API: `hostApiRecords()`, `GET /v1/runs/{id}?wait=10` (args, results, notes and all);
 * - none (`--upstream` without a host token): the telemetry module's records, folded from Neo's
 *   `neo.run` and operation spans (`server/telemetry/`).
 * Both give the same `NeoRunRecord`, attached by the store's one writer, `attachNeoRun`, and kept
 * in the log. A run whose record cannot be read (an older image, the image stopped) keeps none.
 */

import type {
  NeoOperation,
  NeoOperationError,
  NeoRunRecord,
  RunSummary,
  RunTraceContext,
} from '../shared/api'
import { hostFetch } from './hostApi'

/** Seconds Neo holds the read until the run is `complete`. */
const WAIT_SECONDS = 10
/** Reads of a record still incomplete after `WAIT_SECONDS` (an operation the script left running). */
const MAX_READS = 3

export type Raw = Record<string, unknown>

/** Neo's `Run` JSON (openapi.yaml) as the viewer's `NeoRunRecord`. */
export function toNeoRunRecord(raw: Raw): NeoRunRecord {
  const startedAt = numberOr(raw.startedAt, 0)
  const endedAt = numberOr(raw.endedAt, null)
  const operations = Array.isArray(raw.operations)
    ? (raw.operations as Raw[])
    : []
  return {
    status: stringOr(raw.status, 'unknown'),
    effects: stringOr(raw.effects, null),
    startedAt,
    endedAt,
    durationMs: endedAt === null ? null : endedAt - startedAt,
    complete: raw.complete === true,
    tabs: Array.isArray(raw.tabs)
      ? raw.tabs.filter((tab): tab is number => typeof tab === 'number')
      : [],
    error: toError(raw.error),
    operations: operations.map((operation) =>
      toOperation(operation, startedAt),
    ),
    traceGaps: numberOr(raw.traceGaps, 0) ?? 0,
    ...(typeof raw.values === 'string' ? { values: raw.values } : {}),
  }
}

function toOperation(raw: Raw, runStartedAt: number): NeoOperation {
  const id = stringOr(raw.id, '')
  const site = raw.site as Raw | undefined
  return {
    id,
    seq: Number(id.slice(id.lastIndexOf('.') + 1)) || 0,
    parentId: stringOr(raw.parentId, null),
    name: stringOr(raw.name, ''),
    kind: stringOr(raw.kind, ''),
    effect: stringOr(raw.effect, ''),
    site:
      site && typeof site.line === 'number' && typeof site.column === 'number'
        ? { line: site.line, column: site.column }
        : null,
    startOffsetMs: numberOr(raw.startedAt, runStartedAt) - runStartedAt,
    durationMs: numberOr(raw.durationMs, null),
    tabId: numberOr(raw.tabId, null),
    args: raw.args ?? null,
    argsTruncated: raw.argsTruncated === true,
    result: raw.result ?? null,
    resultTruncated: raw.resultTruncated === true,
    outcome: stringOr(raw.outcome, null),
    delivery: stringOr(raw.delivery, null),
    error: toError(raw.error),
    notes: Array.isArray(raw.notes)
      ? raw.notes.filter((note): note is string => typeof note === 'string')
      : [],
    hasScreenshot: raw.hasScreenshot === true,
    ...(typeof raw.values === 'string' ? { values: raw.values } : {}),
  }
}

function toError(raw: unknown): NeoOperationError | null {
  if (typeof raw !== 'object' || raw === null) return null
  const error = raw as Raw
  return {
    owner: stringOr(error.owner, ''),
    code: stringOr(error.code, ''),
    message: stringOr(error.message, ''),
    retryable: error.retryable === true,
    ...(typeof error.hostCode === 'string' ? { hostCode: error.hostCode } : {}),
    ...(typeof error.operationId === 'string'
      ? { operationId: error.operationId }
      : {}),
  }
}

function stringOr<T extends string | null>(
  value: unknown,
  fallback: T,
): string | T {
  return typeof value === 'string' ? value : fallback
}

function numberOr<T extends number | null>(
  value: unknown,
  fallback: T,
): number | T {
  return typeof value === 'number' ? value : fallback
}

/** Reads Neo's record of a run; null when the host API cannot give it. */
async function readNeoRun(
  neoRunId: string,
  waitSeconds = WAIT_SECONDS,
): Promise<NeoRunRecord | null> {
  const response = await hostFetch(
    `/v1/runs/${encodeURIComponent(neoRunId)}?wait=${waitSeconds}`,
  )
  if (!response.ok) return null
  return toNeoRunRecord((await response.json()) as Raw)
}

/** What the joiner knows about an ended run when it asks for its record. */
export interface RunKey {
  /** From the reply's `_meta["com.browseros.neo/run"].id`; null when no reply came. */
  neoRunId: string | null
  /** The context the run went to Neo with; a minted one (`injected`) binds a run with no reply. */
  trace: RunTraceContext | null
}

/** Where Neo's records of runs come from: one source per viewer process. */
export interface RunRecordSource {
  /** Whether a run with this key can have a record from this source at all. */
  canRead(key: RunKey): boolean
  /**
   * The run's record, once complete or after a bounded wait (an incomplete one then, when there
   * is something to show); null when this source has none. Reading again gives the same record or
   * a superset of it, never fewer operations.
   */
  read(key: RunKey, signal: AbortSignal): Promise<NeoRunRecord | null>
  /**
   * A run's record grew after its reads ended (its `neo.run` span came late); the joiner reads it
   * once more. The host API never does this: its `wait` covers it.
   */
  onLateCompletion?(listener: (key: RunKey) => void): () => void
}

/** Today's read: `GET /v1/runs/{id}?wait=10` on Neo's host API. */
export function hostApiRecords(
  read: (neoRunId: string) => Promise<NeoRunRecord | null> = readNeoRun,
): RunRecordSource {
  return {
    canRead: (key) => key.neoRunId !== null,
    read: (key) =>
      key.neoRunId === null ? Promise.resolve(null) : read(key.neoRunId),
  }
}

/** Where the joiner reads ended runs and puts the records it read. */
export interface RunRecords {
  subscribe(
    listener: (event: { type: string; run?: RunSummary }) => void,
  ): () => void
  attachNeoRun(runId: string, record: NeoRunRecord): void
  hasNeoRun(runId: string): boolean
}

/** Longest a stopping viewer waits for reads in flight, so Ctrl-C stays quick. */
const DRAIN_MS = 3000
/** Runs a late completion can still find: the most recent, and none from before a Clear. */
const REMEMBERED_RUNS = 1_000

/**
 * Watches the store for runs that ended and joins Neo's record to each from `source`, one read per
 * run at a time. An incomplete record is attached and read again, up to `MAX_READS`; a record that
 * grows after that (`onLateCompletion`) is read once more.
 */
export function joinNeoRuns(
  store: RunRecords,
  source: RunRecordSource,
): {
  settled: (maxMs: number) => Promise<void>
  settledFor: (runId: string, maxMs: number) => Promise<void>
  stop: () => Promise<void>
} {
  const reading = new Map<string, Promise<void>>()
  /** The run each key was read for, so a late completion finds its run. */
  const runsByKey = new Map<string, string>()
  const stopping = new AbortController()
  const join = async (runId: string, key: RunKey) => {
    for (let attempt = 1; attempt <= MAX_READS; attempt++) {
      const record = await source.read(key, stopping.signal).catch(() => null)
      if (!record) return
      store.attachNeoRun(runId, record)
      if (record.complete) return
    }
  }
  const start = (runId: string, key: RunKey) => {
    if (reading.has(runId)) return
    const pending = join(runId, key).finally(() => reading.delete(runId))
    reading.set(runId, pending)
  }
  const unsubscribe = store.subscribe((event) => {
    if (event.type === 'clear') runsByKey.clear()
    const run = event.run
    if (
      event.type !== 'run' ||
      !run ||
      run.status === 'running' ||
      run.status === 'lost'
    )
      return
    const key: RunKey = { neoRunId: run.neoRunId, trace: run.trace ?? null }
    if (!source.canRead(key) || store.hasNeoRun(run.id) || reading.has(run.id))
      return
    runsByKey.set(keyOf(key), run.id)
    if (runsByKey.size > REMEMBERED_RUNS)
      runsByKey.delete(runsByKey.keys().next().value as string)
    start(run.id, key)
  })
  const unlisten =
    source.onLateCompletion?.((key) => {
      const runId = runsByKey.get(keyOf(key))
      if (runId) start(runId, key)
    }) ?? (() => {})
  const inFlight = (maxMs: number) =>
    Promise.race([Promise.allSettled(reading.values()), Bun.sleep(maxMs)])
  return {
    /**
     * Waits (at most `maxMs`) for the reads in flight, and keeps joining: an export taken right
     * after a run ended then holds Neo's record of it rather than none.
     */
    async settled(maxMs) {
      await inFlight(maxMs)
    },
    /**
     * Waits (at most `maxMs`) for the read of one run's record, if one is in flight: Copy of a
     * run that just ended then holds its methods. Returns at once when nothing is being read.
     */
    async settledFor(runId, maxMs) {
      const pending = reading.get(runId)
      if (pending) await Promise.race([pending, Bun.sleep(maxMs)])
    },
    /**
     * Stops joining and waits (at most `DRAIN_MS`) for reads in flight, so a run that ended just
     * before Ctrl-C still gets its record into the log before the final flush.
     */
    async stop() {
      unsubscribe()
      unlisten()
      await inFlight(DRAIN_MS)
      stopping.abort()
    },
  }
}

/** A run's key as text: its Neo run id, else the span id the viewer minted for it. */
function keyOf(key: RunKey): string {
  return key.neoRunId ?? `span:${key.trace?.spanId ?? ''}`
}
