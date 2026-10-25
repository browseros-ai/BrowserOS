import { useEffect, useRef } from 'react'
import { useSearchParams } from 'react-router'
import { type TaskSummary, useSessions } from '@/modules/api/audit.hooks'
import {
  type AgentOption,
  agentOptionsFor,
  type DayGroup,
  groupByDay,
  needsALook,
} from './audit.helpers'
import {
  type AuditFilters,
  filtersToParams,
  type HistoryView,
  paramsToFilters,
} from './audit.search-params'

/**
 * How many extra pages the screen fetches on its own when the filters leave
 * the loaded pages empty. With 100 tasks a page this scans at most 600
 * before showing "No tasks match", so a filter that matches nothing never
 * walks the whole history.
 */
const AUTO_FETCH_CAP = 5

export interface AuditScreenData {
  groups: DayGroup[]
  /** Every task the server returned, before the "Needs a look" filter. */
  loadedCount: number
  agentOptions: AgentOption[]
  isLoading: boolean
  isError: boolean
  hasNextPage: boolean
  isFetchingNextPage: boolean
  fetchNextPage: () => void
  filters: AuditFilters
  hasActiveFilters: boolean
  setAgentFilter: (slug: string | null) => void
  setView: (view: HistoryView) => void
  setSearch: (search: string) => void
  clearFilters: () => void
}

/**
 * History's one data hook. Filters live in the URL so back and forward
 * restore a view. The agent and the search go to the server; "Needs a look"
 * covers two statuses, which the server cannot ask for at once, so it is
 * applied here.
 */
export function useAuditScreenData(): AuditScreenData {
  const [params, setParams] = useSearchParams()
  const filters = paramsToFilters(params)

  const query = useSessions({
    variables: {
      slug: filters.agentSlug ?? undefined,
      search: filters.search || undefined,
      limit: 100,
    },
  })

  const pages = query.data?.pages
  const loaded: TaskSummary[] = (pages ?? []).flatMap((page) => page.items)
  const tasks =
    filters.view === 'attention' ? loaded.filter(needsALook) : loaded

  // The server filters search after reading a page, and "Needs a look" is
  // filtered here, so either can leave a page empty while older matches exist.
  const narrowsPages = filters.view === 'attention' || filters.search !== ''

  // Each filter set gets its own auto-fetch budget.
  const autoFetchCount = useRef(0)
  const filterKey = `${filters.agentSlug ?? ''}|${filters.view}|${filters.search}`
  const previousFilterKey = useRef(filterKey)
  if (previousFilterKey.current !== filterKey) {
    previousFilterKey.current = filterKey
    autoFetchCount.current = 0
  }

  const shouldAutoFetch =
    narrowsPages &&
    pages !== undefined &&
    tasks.length === 0 &&
    Boolean(query.hasNextPage) &&
    !query.isFetchingNextPage &&
    autoFetchCount.current < AUTO_FETCH_CAP

  // Fetching is a side effect of a render that found nothing to show; it
  // cannot run during render, and no event handler sees the empty result.
  useEffect(() => {
    if (!shouldAutoFetch) return
    autoFetchCount.current += 1
    void query.fetchNextPage()
  }, [shouldAutoFetch, query.fetchNextPage])

  const isAutoFetching =
    narrowsPages &&
    tasks.length === 0 &&
    (query.isFetchingNextPage || shouldAutoFetch)

  const update = (patch: Partial<AuditFilters>) => {
    setParams(filtersToParams({ ...filters, ...patch }), { replace: true })
  }

  return {
    groups: groupByDay(tasks, Date.now()),
    loadedCount: loaded.length,
    agentOptions: agentOptionsFor(loaded, filters.agentSlug),
    isLoading: query.isPending || isAutoFetching,
    isError: query.isError,
    hasNextPage: Boolean(query.hasNextPage),
    isFetchingNextPage: query.isFetchingNextPage,
    fetchNextPage: () => {
      void query.fetchNextPage()
    },
    filters,
    hasActiveFilters:
      filters.agentSlug !== null ||
      filters.view !== 'all' ||
      filters.search !== '',
    setAgentFilter: (agentSlug) => update({ agentSlug }),
    setView: (view) => update({ view }),
    setSearch: (search) => update({ search }),
    clearFilters: () => setParams(new URLSearchParams(), { replace: true }),
  }
}
