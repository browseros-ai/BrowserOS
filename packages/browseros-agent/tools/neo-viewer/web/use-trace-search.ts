/**
 * Searches the viewer's received OTLP run slices. Health revisions refresh the same search
 * without blanking it; a changed query or run generation fences replies from before Clear.
 */
import { useEffect, useRef, useState } from 'react'
import { apiRoutes, type RunSummary, type TracesResponse } from '../shared/api'
import type { ExplorerSpanFilter } from './screen/explorer-filter'
import { getJson } from './use-viewer'

type SearchState =
  | { kind: 'loading' }
  | { kind: 'failed' }
  | { kind: 'ready'; answer: TracesResponse; failed: boolean }

interface ReadSequence {
  key: string
  sent: number
  shown: number
}

/** Poll only while the explorer is mounted. Within a search, newer answers win. */
export function useTraceSearch(
  runs: RunSummary[],
  rev: number | undefined,
  traceId: string,
  filter: ExplorerSpanFilter | null,
): SearchState {
  const query = new URLSearchParams()
  if (traceId.trim()) query.set('traceId', traceId.trim())
  if (filter) {
    query.set('method', filter.method)
    query.set('minMs', String(filter.minMs))
    if (filter.maxMs !== null) query.set('maxMs', String(filter.maxMs))
  }
  const url = `${apiRoutes.traces}${query.size ? `?${query}` : ''}`
  // Runs stay in arrival order and Clear creates a fresh log ID. The first run fences that
  // generation while ordinary arrivals preserve the ready results during a refresh.
  const first = runs[0]
  const key = `${url}\n${first ? `${first.id}@${first.sentAt}` : ''}`
  const [read, setRead] = useState<{ key: string; state: SearchState } | null>(
    null,
  )
  const sequence = useRef<ReadSequence | null>(null)

  useEffect(
    () => () => {
      sequence.current = null
    },
    [],
  )
  // biome-ignore lint/correctness/useExhaustiveDependencies: rev triggers a fresh search of received spans.
  useEffect(() => {
    if (sequence.current?.key !== key)
      sequence.current = { key, sent: 0, shown: 0 }
    const requests = sequence.current
    const seq = ++requests.sent
    void getJson<TracesResponse>(url).then(
      (answer) => {
        if (sequence.current !== requests || seq <= requests.shown) return
        requests.shown = seq
        setRead({ key, state: { kind: 'ready', answer, failed: false } })
      },
      () => {
        if (sequence.current !== requests || seq <= requests.shown) return
        requests.shown = seq
        setRead((last) => ({
          key,
          state:
            last?.key === key && last.state.kind === 'ready'
              ? { ...last.state, failed: true }
              : { kind: 'failed' },
        }))
      },
    )
  }, [key, url, rev, runs])
  return read?.key === key ? read.state : { kind: 'loading' }
}
