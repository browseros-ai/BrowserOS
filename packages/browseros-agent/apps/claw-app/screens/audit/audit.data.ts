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

  const update = (patch: Partial<AuditFilters>): void => {
    const next: AuditFilters = { ...filters, ...patch }
    setParams(filtersToParams(next), { replace: true })
  }

  return {
    tasks,
    agentOptions,
    statusOptions: statusOpts,
    siteOptions: siteOpts,
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
  }
}
