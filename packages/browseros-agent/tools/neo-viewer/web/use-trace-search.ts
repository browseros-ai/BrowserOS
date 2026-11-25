/**
 * Searches the viewer's received OTLP run slices. Health revisions refresh the same search
 * without blanking it; a changed query or run generation fences replies from before Clear.
 */
import { useEffect, useRef, useState } from 'react'
import { apiRoutes, type RunSummary, type TracesResponse } from '../shared/api'
import type { ExplorerSpanFilter } from './screen/explorer-filter'
import { getJson } from './use-viewer'

type SearchRead =
  | { kind: 'loading' }
  | { kind: 'failed' }
  | {
      kind: 'ready'
      answer: TracesResponse
      failed: boolean
      loadingMore: boolean
    }

type SearchState = SearchRead & { loadMore: () => void }

interface SearchDepth {
  key: string
  pages: number
  attempt: number
}

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
  const searchParams = query.toString()
  const url = `${apiRoutes.traces}${searchParams.length ? `?${searchParams}` : ''}`
  // Runs stay in arrival order and Clear creates a fresh log ID. The first run fences that
  // generation while ordinary arrivals preserve the ready results during a refresh.
  const first = runs[0]
  const key = `${url}\n${first ? `${first.id}@${first.sentAt}` : ''}`
  const [read, setRead] = useState<{
    key: string
    state: SearchRead
    pages: number
  } | null>(null)
  const [depth, setDepth] = useState<SearchDepth>({
    key,
    pages: 1,
    attempt: 0,
  })
  const pages = depth.key === key ? depth.pages : 1
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
    // Refresh every loaded page from its current first page so new arrivals cannot leave gaps
    // between old cursor boundaries. Keep reading through the last visible run, preserving a
    // selected older row when newer runs push the page boundary down.
    const previous =
      read?.key === key && read.state.kind === 'ready' ? read : null
    const anchor =
      previous?.state.kind === 'ready'
        ? (previous.state.answer.traces.at(-1)?.run.id ?? null)
        : null
    setRead((last) =>
      last?.key === key && last.state.kind === 'ready'
        ? {
            ...last,
            state: { ...last.state, loadingMore: pages > last.pages },
          }
        : last,
    )
    void readPages(url, pages, anchor).then(
      ({ answer, pages: loaded }) => {
        if (sequence.current !== requests || seq <= requests.shown) return
        requests.shown = seq
        setRead({
          key,
          pages: loaded,
          state: { kind: 'ready', answer, failed: false, loadingMore: false },
        })
      },
      () => {
        if (sequence.current !== requests || seq <= requests.shown) return
        requests.shown = seq
        setRead((last) => ({
          key,
          pages: last?.key === key ? last.pages : 0,
          state:
            last?.key === key && last.state.kind === 'ready'
              ? { ...last.state, failed: true, loadingMore: false }
              : { kind: 'failed' },
        }))
      },
    )
  }, [key, url, rev, runs, pages, depth.attempt])
  const loadMore = () => {
    if (
      read?.key !== key ||
      read.state.kind !== 'ready' ||
      read.state.loadingMore ||
      !read.state.answer.nextCursor
    )
      return
    setDepth((last) => ({
      key,
      pages: read.pages + 1,
      attempt: last.attempt + 1,
    }))
  }
  const current: SearchRead =
    read?.key === key ? read.state : { kind: 'loading' }
  return { ...current, loadMore }
}

/** Each HTTP page stays bounded; filters are copied unchanged into every continuation read. */
async function readPages(
  url: string,
  pages: number,
  anchor: string | null,
): Promise<{ answer: TracesResponse; pages: number }> {
  const traces: TracesResponse['traces'] = []
  let cursor: string | null = null
  let loaded = 0
  let reachedAnchor = anchor === null
  do {
    const next: string = cursor
      ? `${url}${url.includes('?') ? '&' : '?'}cursor=${encodeURIComponent(cursor)}`
      : url
    const answer: TracesResponse = await getJson<TracesResponse>(next)
    traces.push(...answer.traces)
    cursor = answer.nextCursor
    loaded++
    reachedAnchor ||= answer.traces.some((row) => row.run.id === anchor)
  } while (cursor && (loaded < pages || !reachedAnchor))
  return {
    answer: { traces, truncated: cursor !== null, nextCursor: cursor },
    pages: loaded,
  }
}
