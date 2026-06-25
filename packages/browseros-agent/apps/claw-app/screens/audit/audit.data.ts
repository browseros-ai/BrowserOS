import { useMemo } from 'react'
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
  now: number
}

/**
 * Single data hook for the audit screen. Reads filters from URL
 * search params so browser back / forward restores prior views; the
 * useTasks infinite query is variables-keyed off the same filter
 * shape so changing a filter starts a fresh paginated stream.
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

  const tasks = (query.data?.pages ?? []).flatMap((p) => p.tasks)
  const now = Date.now()

  const update = (patch: Partial<AuditFilters>): void => {
    const next: AuditFilters = { ...filters, ...patch }
    setParams(filtersToParams(next), { replace: true })
  }

  return {
    tasks,
    agentOptions: agentChipsFor(tasks),
    statusOptions: statusOptionsOf(tasks),
    siteOptions: siteOptionsOf(tasks),
    isLoading: query.isPending,
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
    now,
  }
}
