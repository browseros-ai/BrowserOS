import { useSearchParams } from 'react-router'
import { useSessions } from '@/modules/api/audit.hooks'
import {
  type AgentOption,
  agentOptionsFor,
  type DayGroup,
  groupByDay,
} from './audit.helpers'
import {
  type AuditFilters,
  filtersToParams,
  type HistoryView,
  paramsToFilters,
} from './audit.search-params'

/** Tasks per page; scrolling near the end loads the next page. */
export const HISTORY_PAGE_SIZE = 20

export interface AuditScreenData {
  groups: DayGroup[]
  /** Tasks loaded so far across pages. */
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
 * restore a view, and every filter runs on the server before pagination, so
 * each page holds a full 20 matching tasks.
 */
export function useAuditScreenData(): AuditScreenData {
  const [params, setParams] = useSearchParams()
  const filters = paramsToFilters(params)

  const query = useSessions({
    variables: {
      slug: filters.agentSlug ?? undefined,
      needsAttention: filters.view === 'attention' || undefined,
      search: filters.search || undefined,
      limit: HISTORY_PAGE_SIZE,
    },
  })

  const tasks = (query.data?.pages ?? []).flatMap((page) => page.items)
  const update = (patch: Partial<AuditFilters>) => {
    setParams(filtersToParams({ ...filters, ...patch }), { replace: true })
  }

  return {
    groups: groupByDay(tasks, Date.now()),
    loadedCount: tasks.length,
    agentOptions: agentOptionsFor(tasks, filters.agentSlug),
    isLoading: query.isPending,
    isError: query.isError,
    hasNextPage: Boolean(query.hasNextPage),
    isFetchingNextPage: query.isFetchingNextPage,
    fetchNextPage: () => {
      if (query.hasNextPage && !query.isFetchingNextPage) {
        void query.fetchNextPage()
      }
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
