/**
 * The page's data: every run since the last Clear, and the log's totals, from the viewer's API,
 * kept current by `/api/events`. Each event carries the new state of one run (or of the log, or a
 * clear) and its change number.
 *
 * The stream opens first and every (re)open takes a fresh snapshot, so no change falls between a
 * snapshot and the stream. A snapshot can still land after events that are newer than it: the
 * page keeps the events since the stream opened and re-applies those numbered above the
 * snapshot's `version`.
 */

import { useCallback, useEffect, useRef, useState } from 'react'
import type {
  LogSummary,
  RunDetail,
  RunResponse,
  RunSummary,
  RunsResponse,
  ViewerEvent,
  ViewerInfo,
} from '../shared/api'
import { apiRoutes } from '../shared/api'

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

export interface ViewerState {
  info: ViewerInfo | null
  /** Every run since the last Clear, in arrival order. */
  runs: RunSummary[]
  log: LogSummary | null
  /** Empties the log (the server keeps the previous one as `log.prev.jsonl`). */
  clear: () => Promise<void>
  error: string | null
}

export function useViewer(): ViewerState {
  const [info, setInfo] = useState<ViewerInfo | null>(null)
  const [state, setState] = useState<{
    runs: RunSummary[]
    log: LogSummary | null
  }>({ runs: [], log: null })
  const [error, setError] = useState<string | null>(null)
  // Bumped each time the event stream opens; the snapshot below is taken after it.
  const [opened, setOpened] = useState(0)
  const recent = useRef<ViewerEvent[]>([])

  useEffect(() => {
    getJson<ViewerInfo>(apiRoutes.info).then(setInfo, (cause) =>
      setError(String(cause)),
    )
  }, [])

  useEffect(() => {
    const events = new EventSource(apiRoutes.events)
    events.onopen = () => {
      recent.current = []
      setOpened((n) => n + 1)
    }
    events.onmessage = (message) => {
      const event = JSON.parse(message.data) as ViewerEvent
      recent.current.push(event)
      if (recent.current.length > KEPT_EVENTS) {
        recent.current.splice(0, recent.current.length - KEPT_EVENTS)
      }
      setState((current) => apply(current, event))
    }
    return () => events.close()
  }, [])

  useEffect(() => {
    if (opened === 0) return
    getJson<RunsResponse>(apiRoutes.runs).then(
      ({ runs, log, version }) => {
        let next: { runs: RunSummary[]; log: LogSummary | null } = { runs, log }
        for (const event of recent.current) {
          if (event.version > version) next = apply(next, event)
        }
        setState(next)
      },
      (cause) => setError(String(cause)),
    )
  }, [opened])

  const clear = useCallback(async () => {
    const response = await fetch(apiRoutes.clear, { method: 'POST' })
    if (!response.ok) setError(`${apiRoutes.clear}: HTTP ${response.status}`)
    // The stream's `clear` event empties the list, here and on any other open page.
  }, [])

  return { info, runs: state.runs, log: state.log, clear, error }
}

/** One run in full, refetched whenever its summary changes (it was still running). */
export function useRunDetail(run: RunSummary, open: boolean): RunDetail | null {
  const [detail, setDetail] = useState<RunDetail | null>(null)
  // biome-ignore lint/correctness/useExhaustiveDependencies: a new status means a new reply to fetch.
  useEffect(() => {
    if (!open) return
    let current = true
    getJson<RunResponse>(apiRoutes.run(run.id)).then(
      (found) => {
        if (current) setDetail(found.run)
      },
      () => {},
    )
    return () => {
      current = false
    }
  }, [run.id, run.status, open])
  return detail
}
