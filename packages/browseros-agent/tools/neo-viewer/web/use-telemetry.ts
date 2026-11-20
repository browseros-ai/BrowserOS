/**
 * The page's two reads of Neo's telemetry beyond health (`useViewer`): the Health view's metrics
 * (`GET /api/metrics`) and one run's trace (`GET /api/runs/:id/trace`). Neither has an event of
 * its own: both follow `TelemetryHealth.rev`, which rises with every metric sample and span batch
 * the viewer accepts (at most once a second on the stream), and read again when it moves.
 *
 * Each keeps what it read last while it reads again, and starts a second view (the Trace row
 * and the sidebar's trace, a reopened Health view) from that, so nothing blanks or flickers as
 * numbers move. A read that lands after a newer one is dropped.
 */

import { useEffect, useRef, useState } from 'react'
import {
  apiRoutes,
  type MetricsResponse,
  type MetricsView,
  type RunSummary,
  type RunTrace,
  type RunTraceResponse,
} from '../shared/api'
import { getJson } from './use-viewer'

/** The last metrics read, so a reopened Health view starts from them instead of empty. */
let lastMetrics: MetricsView | null | undefined

/**
 * Neo's metrics from the viewer's live source while `open`, read again whenever `rev` moves.
 * `undefined` until the first read answers; `null` when the viewer has no live source.
 */
export function useMetrics(
  open: boolean,
  rev: number | undefined,
): MetricsView | null | undefined {
  const [metrics, setMetrics] = useState(lastMetrics)
  // Reads are numbered as sent. A new `rev` does not cancel the read still out (on a slow
  // answer every read would be cancelled); a read older than the one shown is dropped instead.
  const reads = useRef({ sent: 0, shown: 0 })
  // biome-ignore lint/correctness/useExhaustiveDependencies: `rev` is the trigger: a new sample to read.
  useEffect(() => {
    if (!open) return
    const seq = ++reads.current.sent
    getJson<MetricsResponse>(apiRoutes.metrics).then(
      (answer) => {
        if (seq <= reads.current.shown) return
        reads.current.shown = seq
        lastMetrics = answer.metrics
        setMetrics(answer.metrics)
      },
      // A failed read leaves the numbers shown as they were.
      () => {},
    )
  }, [open, rev])
  return metrics
}

/**
 * After a run ends, how long its trace may still grow: Neo exports a span about half a second
 * after it ends. A trace read once this is over is final, so a run from before telemetry (or
 * whose spans never came) costs no request per health change.
 */
export const TRACE_WAIT_MS = 30_000

/** A trace as read, and when: whether a read can still be outgrown depends on when it was made. */
interface TraceRead {
  trace: RunTrace | null
  /** Epoch ms of the read. */
  at: number
}

/** The last trace read per run (ids restart after a Clear, so keyed with the send time too). */
const readTraces = new Map<string, TraceRead>()
const READ_TRACES = 50

function traceKey(run: RunSummary): string {
  return `${run.id}@${run.sentAt}`
}

function rememberTrace(run: RunSummary, read: TraceRead): void {
  const key = traceKey(run)
  readTraces.delete(key)
  readTraces.set(key, read)
  const oldest = readTraces.keys().next().value
  if (readTraces.size > READ_TRACES && oldest !== undefined)
    readTraces.delete(oldest)
}

/** Reads in flight, so the Trace row and the sidebar asking at one `rev` share one request. */
const reading = new Map<string, Promise<RunTrace | null | undefined>>()

/** The run's trace; undefined when the read failed (the run is gone after a Clear, a 404). */
function readTrace(
  run: RunSummary,
  rev: number | undefined,
): Promise<RunTrace | null | undefined> {
  const key = `${traceKey(run)}#${rev ?? ''}`
  const out = reading.get(key)
  if (out) return out
  const read = getJson<RunTraceResponse>(apiRoutes.runTrace(run.id))
    .then((answer) => answer.trace)
    .catch(() => undefined)
    .finally(() => reading.delete(key))
  reading.set(key, read)
  return read
}

/**
 * When a run stopped being able to get spans: when its reply passed back; for one that ended
 * with no reply (`lost`: the viewer stopped first), when it was sent; null while it runs.
 */
function endedAt(run: RunSummary): number | null {
  if (run.status === 'running') return null
  return Date.parse(run.answeredAt ?? run.sentAt)
}

/** Whether a run's trace may still grow at `now`: incomplete, and running or ended within the wait. */
export function traceMayGrow(
  run: RunSummary,
  trace: RunTrace | null | undefined,
  now: number,
): boolean {
  if (trace?.complete) return false
  const ended = endedAt(run)
  return ended === null || now - ended < TRACE_WAIT_MS
}

/**
 * A read nothing can outgrow: the trace is complete, or it was read after the run's wait was
 * over. A partial read made earlier is never final, however long ago it was made: the trace is
 * read again the next time it is shown.
 */
function settled(run: RunSummary, read: TraceRead | undefined): boolean {
  if (!read) return false
  if (read.trace?.complete) return true
  const ended = endedAt(run)
  return ended !== null && read.at - ended >= TRACE_WAIT_MS
}

/** The later of two reads of one run (the hook's own, and one another view made since). */
function later(
  a: TraceRead | undefined,
  b: TraceRead | undefined,
): TraceRead | undefined {
  if (!a) return b
  if (!b) return a
  return b.at > a.at ? b : a
}

/**
 * One run's trace while `open`: `undefined` until the first read answers, null when no span of
 * the run has arrived. Read when shown, and again whenever `rev` moves, until a read is final
 * (`settled`): the trace is complete (`neo.run` has arrived, so it never changes), or it was read
 * after the run's wait was over.
 */
export function useRunTrace(
  run: RunSummary,
  open: boolean,
  rev: number | undefined,
): RunTrace | null | undefined {
  const key = traceKey(run)
  const [own, setOwn] = useState<{ key: string; read: TraceRead } | null>(null)
  const latest = later(
    own?.key === key ? own.read : undefined,
    readTraces.get(key),
  )
  const done = settled(run, latest)
  // biome-ignore lint/correctness/useExhaustiveDependencies: `rev` and the run's status are the triggers: spans may have arrived.
  useEffect(() => {
    if (!open || done) return
    let current = true
    void readTrace(run, rev).then((trace) => {
      if (!current || trace === undefined) return
      const read = { trace, at: Date.now() }
      rememberTrace(run, read)
      setOwn({ key, read })
    })
    return () => {
      current = false
    }
  }, [key, open, rev, run.status, done])
  return latest?.trace
}
