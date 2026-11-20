/**
 * The page's data: every run since the last Clear, and the log's totals, from the viewer's API,
 * kept current by `/api/events`. Every viewer tab of the origin hears that one stream
 * (`viewer-events.ts`): one tab holds it and relays it to the others. Each event carries the new
 * state of one run (or of the log, or a clear) and its change number.
 *
 * Every (re)open of the stream this tab hears takes a fresh snapshot, so no change falls between
 * a snapshot and the stream. A snapshot can still land after events that are newer than it: the
 * page keeps the events since the stream opened and re-applies those numbered above the
 * snapshot's `version`.
 *
 * Neo's health (`TelemetryHealth`) rides the same stream and snapshot but is not a change of the
 * list: it has no `version` and never joins that replay. It is one full state with its own `rev`,
 * so the page keeps the highest `rev` it heard (`keepNewer`), from events and snapshots alike.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import type {
  ExportResponse,
  LogSummary,
  RunDetail,
  RunResponse,
  RunSummary,
  RunsResponse,
  TelemetryHealth,
  ViewerEvent,
  ViewerInfo,
} from '../shared/api'
import { apiRoutes } from '../shared/api'
import { listenToViewerEvents } from './viewer-events'

/** Events kept for re-applying over a late snapshot; older ones are long since in any snapshot. */
const KEPT_EVENTS = 2000

async function getJson<T>(path: string): Promise<T> {
  const response = await fetch(path)
  if (!response.ok) throw new Error(`${path}: HTTP ${response.status}`)
  return (await response.json()) as T
}

function upsert(runs: RunSummary[], run: RunSummary): RunSummary[] {
  const index = runs.findIndex((candidate) => candidate.id === run.id)
  const next =
    index === -1
      ? [...runs, run]
      : runs.map((candidate, i) => (i === index ? run : candidate))
  return next.sort((a, b) => a.seq - b.seq)
}

/** The list and totals after one event: a clear empties the list, a run updates it, a log event the totals. */
function apply(
  state: { runs: RunSummary[]; log: LogSummary | null },
  event: ViewerEvent,
) {
  if (event.type === 'clear') return { runs: [], log: event.log }
  if (event.type === 'log') return { ...state, log: event.log }
  return { ...state, runs: upsert(state.runs, event.run) }
}

/**
 * The health to show after `next` arrives: the one with the higher `rev`. Equal or lower is an
 * older state (a snapshot read before an event it crossed), and null says nothing new.
 */
export function keepNewer(
  current: TelemetryHealth | null,
  next: TelemetryHealth | null,
): TelemetryHealth | null {
  if (next === null) return current
  return current === null || next.rev > current.rev ? next : current
}

export interface ViewerState {
  info: ViewerInfo | null
  /** Every run since the last Clear, in arrival order. */
  runs: RunSummary[]
  log: LogSummary | null
  /** Neo's health; null until Neo's telemetry first reaches the viewer (never, for older images). */
  health: TelemetryHealth | null
  /** Empties the log (the server keeps the previous one as `log.prev.jsonl`). */
  clear: () => Promise<void>
  /** Writes every run since the last Clear to a new folder; null when the server refused. */
  exportRuns: () => Promise<ExportResponse | null>
  error: string | null
}

export function useViewer(): ViewerState {
  const [info, setInfo] = useState<ViewerInfo | null>(null)
  const [state, setState] = useState<{
    runs: RunSummary[]
    log: LogSummary | null
  }>({ runs: [], log: null })
  const [health, setHealth] = useState<TelemetryHealth | null>(null)
  const [error, setError] = useState<string | null>(null)
  // Bumped each time the event stream opens; the snapshot below is taken after it.
  const [opened, setOpened] = useState(0)
  const recent = useRef<ViewerEvent[]>([])
  // The newest health heard since the stream last opened. `rev` counts within one viewer
  // process, so each opening compares only with what it heard itself: a viewer restarted behind
  // the stream starts again from rev 1, and its health still shows. Until the opening's first
  // health arrives, the one shown stays, so a reconnect never blanks it.
  const healthSinceOpen = useRef<TelemetryHealth | null>(null)

  useEffect(() => {
    getJson<ViewerInfo>(apiRoutes.info).then(setInfo, (cause) =>
      setError(String(cause)),
    )
  }, [])

  useEffect(
    () =>
      listenToViewerEvents({
        opened() {
          recent.current = []
          healthSinceOpen.current = null
          setOpened((n) => n + 1)
        },
        event(event) {
          // Health is not a change of the list: it has no `version` and is never replayed.
          if (event.type === 'health') {
            const next = keepNewer(healthSinceOpen.current, event.health)
            healthSinceOpen.current = next
            setHealth(next)
            return
          }
          recent.current.push(event)
          if (recent.current.length > KEPT_EVENTS) {
            recent.current.splice(0, recent.current.length - KEPT_EVENTS)
          }
          setState((current) => apply(current, event))
        },
      }),
    [],
  )

  useEffect(() => {
    if (opened === 0) return
    // A later opening emptied `recent` and took its own snapshot: this one, landing after it,
    // would roll the list back to before that opening.
    let current = true
    getJson<RunsResponse>(apiRoutes.runs).then(
      ({ runs, log, version, health: snapshotHealth }) => {
        if (!current) return
        let next: { runs: RunSummary[]; log: LogSummary | null } = { runs, log }
        for (const event of recent.current) {
          if (event.version > version) next = apply(next, event)
        }
        setState(next)
        // A snapshot with no health, when this opening heard none either, is a viewer with no
        // telemetry (one restarted behind the stream): nothing to show.
        const newest = keepNewer(
          healthSinceOpen.current,
          snapshotHealth ?? null,
        )
        healthSinceOpen.current = newest
        setHealth(newest)
      },
      (cause) => {
        if (current) setError(String(cause))
      },
    )
    return () => {
      current = false
    }
  }, [opened])

  const clear = useCallback(async () => {
    const response = await fetch(apiRoutes.clear, { method: 'POST' })
    if (!response.ok) setError(`${apiRoutes.clear}: HTTP ${response.status}`)
    // The stream's `clear` event empties the list, here and on any other open page.
  }, [])

  const exportRuns = useCallback(async () => {
    try {
      const response = await fetch(apiRoutes.export, { method: 'POST' })
      if (!response.ok) {
        const body = (await response.json().catch(() => null)) as {
          error?: string
        } | null
        setError(body?.error ?? `${apiRoutes.export}: HTTP ${response.status}`)
        return null
      }
      return (await response.json()) as ExportResponse
    } catch (cause) {
      // The viewer stopped, or answered with something that is not an export.
      setError(`${apiRoutes.export}: ${String(cause)}`)
      return null
    }
  }, [])

  return {
    info,
    runs: state.runs,
    log: state.log,
    health,
    clear,
    exportRuns,
    error,
  }
}

/**
 * The last detail read for each recent run, keyed by its id and the time it was sent (ids start
 * again after a Clear). A second view of a run (the right panel, Open full) starts from what its
 * block already read, and keeps it when a refetch fails.
 */
const readDetails = new Map<string, RunDetail>()
const READ_DETAILS = 50

function detailKey(run: RunSummary): string {
  return `${run.id}@${run.sentAt}`
}

function remember(run: RunSummary, detail: RunDetail): void {
  readDetails.delete(detailKey(run))
  readDetails.set(detailKey(run), detail)
  const oldest = readDetails.keys().next().value
  if (readDetails.size > READ_DETAILS && oldest !== undefined)
    readDetails.delete(oldest)
}

/** One run in full, refetched whenever its summary changes (it was still running). */
export function useRunDetail(run: RunSummary, open: boolean): RunDetail | null {
  const [detail, setDetail] = useState<RunDetail | null>(
    () => readDetails.get(detailKey(run)) ?? null,
  )
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new status means a new reply, a new record count or duration means Neo's record joined.
  useEffect(() => {
    if (!open) return
    let current = true
    getJson<RunResponse>(apiRoutes.run(run.id)).then(
      (found) => {
        // A newer read replaced this one: it must not overwrite the cache or the view.
        if (!current) return
        remember(run, found.run)
        setDetail(found.run)
      },
      // The last detail read stays: a failed read leaves the view as it was.
      () => {},
    )
    return () => {
      current = false
    }
  }, [run.id, run.status, run.operationCount, run.neoDurationMs, open])
  return detail?.id === run.id
    ? detail
    : (readDetails.get(detailKey(run)) ?? detail)
}
