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
    fetch(apiRoutes.metrics)
      .then((response) =>
        response.ok ? (response.json() as Promise<MetricsResponse>) : null,
      )
      .then(
        (answer) => {
          if (!answer || seq <= reads.current.shown) return
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
 * After a run ends, how long its trace is read again as spans arrive. Neo exports a span about
 * half a second after it ends; a trace still empty or partial past this stays as read, so a run
 * from before telemetry (or whose spans never came) costs no request per health change.
 */
export const TRACE_WAIT_MS = 30_000

/** The last trace read per run (ids restart after a Clear, so keyed with the send time too). */
const readTraces = new Map<string, RunTrace | null>()
const READ_TRACES = 50

function traceKey(run: RunSummary): string {
  return `${run.id}@${run.sentAt}`
}

function rememberTrace(run: RunSummary, trace: RunTrace | null): void {
  const key = traceKey(run)
  readTraces.delete(key)
  readTraces.set(key, trace)
  const oldest = readTraces.keys().next().value
  if (readTraces.size > READ_TRACES && oldest !== undefined)
    readTraces.delete(oldest)
}

/** Reads in flight, so the Trace row and the sidebar asking at one `rev` share one request. */
const reading = new Map<string, Promise<RunTrace | null | undefined>>()

function readTrace(
  run: RunSummary,
  rev: number | undefined,
): Promise<RunTrace | null | undefined> {
  const key = `${traceKey(run)}#${rev ?? ''}`
  const out = reading.get(key)
  if (out) return out
  const read = fetch(apiRoutes.runTrace(run.id))
    .then((response) => {
      // 404: the run is gone (a Clear crossed the read); keep what was shown.
      if (!response.ok) return undefined
      return (response.json() as Promise<RunTraceResponse>).then(
        (answer) => answer.trace,
      )
    })
    .catch(() => undefined)
    .finally(() => reading.delete(key))
  reading.set(key, read)
  return read
}

/** Whether a run's trace may still grow: running, or ended within `TRACE_WAIT_MS`. */
export function traceMayGrow(
  run: RunSummary,
  trace: RunTrace | null | undefined,
  now: number,
): boolean {
  if (trace?.complete) return false
  if (run.status === 'running' || run.answeredAt === null) return true
  return now - Date.parse(run.answeredAt) < TRACE_WAIT_MS
}

/**
 * One run's trace while `open`: `undefined` until the first read answers, null when no span of
 * the run has arrived. Read again when `rev` moves, until the trace is complete (`neo.run` has
 * arrived, so it never changes) or the run ended long enough ago that no more spans will come.
 */
export function useRunTrace(
  run: RunSummary,
  open: boolean,
  rev: number | undefined,
): RunTrace | null | undefined {
  const key = traceKey(run)
  const [read, setRead] = useState<{
    key: string
    trace: RunTrace | null | undefined
  }>(() => ({ key, trace: readTraces.get(key) }))
  const shown = read.key === key ? read.trace : readTraces.get(key)
  const settled = shown !== undefined && !traceMayGrow(run, shown, Date.now())
  // biome-ignore lint/correctness/useExhaustiveDependencies: `rev` and the run's status are the triggers: spans may have arrived.
  useEffect(() => {
    if (!open || settled) return
    let current = true
    void readTrace(run, rev).then((trace) => {
      if (!current || trace === undefined) return
      rememberTrace(run, trace)
      setRead({ key, trace })
    })
    return () => {
      current = false
    }
  }, [key, open, rev, run.status, settled])
  return shown
}
