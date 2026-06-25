import { useEffect, useMemo, useRef } from 'react'
import { useSearchParams } from 'react-router'
import {
  type TaskStatus,
  type TaskSummary,
  useTasks,
} from '@/modules/api/audit.hooks'
import {
  type AgentChip,
  agentChipsFor,
  siteOptions as siteOptionsOf,
  statusOptions as statusOptionsOf,
} from './audit.helpers'
import {
  type AuditFilters,
  filtersToParams,
  paramsToFilters,
} from './audit.search-params'

/**
 * Hard cap on how many extra pages this hook will auto-fetch when
 * the JS-layer filters (status / site / search / since) shrink the
 * current page to zero matches. With `limit: 100` per page this caps
 * the scan at `(1 + cap) * 100 = 600` sessions before the operator
 * sees the empty state. Prevents a no-match filter from scanning the
 * entire audit log.
 */
const AUTO_FETCH_CAP = 5

export interface AuditScreenData {
  tasks: TaskSummary[]
  agentOptions: AgentChip[]
  statusOptions: { status: TaskStatus; count: number }[]
  siteOptions: { site: string; count: number }[]
  isLoading: boolean
  isError: boolean
  hasNextPage: boolean
  isFetchingNextPage: boolean
  fetchNextPage: () => void
  filters: AuditFilters
  setAgentFilter: (agentId: string | null) => void
  setStatusFilter: (status: TaskStatus | null) => void
  setSiteFilter: (site: string | null) => void
  setSearch: (q: string) => void
  setSort: (sort: AuditFilters['sort']) => void
}

/**
 * Single data hook for the audit screen. Reads filters from URL
 * search params so browser back / forward restores prior views; the
 * useTasks infinite query is variables-keyed off the same filter
 * shape so changing a filter starts a fresh paginated stream.
 *
 * Every returned value (`tasks`, `agentOptions`, etc.) is memoised so
 * the consumer can pass them to `useReactTable` without triggering a
 * re-process on every render. tanstack-table requires `data` to be a
 * stable reference; an inline `flatMap` here would create a new array
 * each render and put the table in a render storm.
 *
 * Auto-pagination on filtered emptiness: status / site / search / since
 * filter in JS after the SQL page is fetched. If the first 100 sessions
 * are all wrong-status (etc.) the response is `{tasks: [], nextCursor}`
 * and the operator sees "No tasks match these filters" even when
 * matches exist further back. This hook detects that case and chains
 * `fetchNextPage()` automatically up to `AUTO_FETCH_CAP` pages so the
 * operator gets the first real match or a true empty state, not a
 * premature one.
 */
export function useAuditScreenData(): AuditScreenData {
  const [params, setParams] = useSearchParams()
  const filters = useMemo(() => paramsToFilters(params), [params])

  const query = useTasks({
    variables: {
      agentId: filters.agentId ?? undefined,
      status: filters.status ?? undefined,
      site: filters.site ?? undefined,
      search: filters.search || undefined,
      limit: 100,
    },
  })

  const pages = query.data?.pages
  const tasks = useMemo(() => (pages ?? []).flatMap((p) => p.tasks), [pages])
  const agentOptions = useMemo(() => agentChipsFor(tasks), [tasks])
  const statusOpts = useMemo(() => statusOptionsOf(tasks), [tasks])
  const siteOpts = useMemo(() => siteOptionsOf(tasks), [tasks])

  // Filters applied in JS by the server-side tasks deriver. These are
  // the ones that can shrink a SQL page to zero matches.
  const hasJsLevelFilter =
    filters.status !== null ||
    filters.site !== null ||
    filters.search.length > 0

  // Reset the auto-fetch counter whenever the filter combo changes.
  // Each variable set gets its own budget; the counter is a poor-man's
  // per-query state we cannot get from react-query directly.
  const autoFetchCount = useRef(0)
  const filterKey = useMemo(
    () =>
      `${filters.agentId ?? ''}|${filters.status ?? ''}|${filters.site ?? ''}|${filters.search}`,
    [filters.agentId, filters.status, filters.site, filters.search],
  )
  const prevFilterKey = useRef(filterKey)
  if (prevFilterKey.current !== filterKey) {
    prevFilterKey.current = filterKey
    autoFetchCount.current = 0
  }

  const shouldAutoPaginate =
    hasJsLevelFilter &&
    pages !== undefined &&
    tasks.length === 0 &&
    Boolean(query.hasNextPage) &&
    !query.isFetchingNextPage &&
    autoFetchCount.current < AUTO_FETCH_CAP

  useEffect(() => {
    if (!shouldAutoPaginate) return
    autoFetchCount.current += 1
    void query.fetchNextPage()
  }, [shouldAutoPaginate, query.fetchNextPage])

  const isAutoPaginating =
    hasJsLevelFilter &&
    tasks.length === 0 &&
    (query.isFetchingNextPage || shouldAutoPaginate)

  const update = (patch: Partial<AuditFilters>): void => {
    const next: AuditFilters = { ...filters, ...patch }
    setParams(filtersToParams(next), { replace: true })
  }

  return {
    tasks,
    agentOptions,
    statusOptions: statusOpts,
    siteOptions: siteOpts,
    isLoading: query.isPending || isAutoPaginating,
    isError: query.isError,
    hasNextPage: Boolean(query.hasNextPage),
    isFetchingNextPage: query.isFetchingNextPage,
    fetchNextPage: () => {
      void query.fetchNextPage()
    },
    filters,
    setAgentFilter: (agentId) => update({ agentId }),
    setStatusFilter: (status) => update({ status }),
    setSiteFilter: (site) => update({ site }),
    setSearch: (search) => update({ search }),
    setSort: (sort) => update({ sort }),
  }
}
