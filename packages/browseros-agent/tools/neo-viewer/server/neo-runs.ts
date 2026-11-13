/**
 * Joins Neo's own record of each run (its host API's `GET /v1/runs/{id}`) to the proxy's record.
 *
 *   proxy: a run's reply passes ─▶ store emits the ended run, with `neoRunId` from the reply's _meta
 *   joiner: GET /v1/runs/{neoRunId}?wait=10 (hostFetch) ─▶ toNeoRunRecord ─▶ store.attachNeoRun
 *
 * The proxy has what the agent sent and read; only Neo has the operations (each call's name,
 * site, timing, args, result, notes, delivery, error). The record is read once the reply is in;
 * `wait` holds the request until Neo has stored the run's last operation. A run whose record
 * cannot be read (no host API, image stopped) keeps a null record.
 */

import type {
  NeoOperation,
  NeoOperationError,
  NeoRunRecord,
  RunSummary,
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

/**
 * Watches the store for runs that ended with a Neo run id and joins Neo's record to each, one read
 * per run at a time. An incomplete record is attached and read again, up to `MAX_READS`.
 */
export function joinNeoRuns(
  store: RunRecords,
  read = readNeoRun,
): {
  settled: (maxMs: number) => Promise<void>
  settledFor: (runId: string, maxMs: number) => Promise<void>
  stop: () => Promise<void>
} {
  const reading = new Map<string, Promise<void>>()
  const join = async (runId: string, neoRunId: string) => {
    for (let attempt = 1; attempt <= MAX_READS; attempt++) {
      const record = await read(neoRunId).catch(() => null)
      if (!record) return
      store.attachNeoRun(runId, record)
      if (record.complete) return
    }
  }
  const unsubscribe = store.subscribe((event) => {
    const run = event.run
    if (
      event.type !== 'run' ||
      !run ||
      run.status === 'running' ||
      run.status === 'lost'
    )
      return
    if (!run.neoRunId || store.hasNeoRun(run.id) || reading.has(run.id)) return
    const pending = join(run.id, run.neoRunId).finally(() =>
      reading.delete(run.id),
    )
    reading.set(run.id, pending)
  })
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
      await inFlight(DRAIN_MS)
    },
  }
}
