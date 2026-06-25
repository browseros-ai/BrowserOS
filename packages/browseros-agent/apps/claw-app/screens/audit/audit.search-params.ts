import type { TaskStatus } from '@/modules/api/audit.hooks'

export interface AuditFilters {
  agentId: string | null
  status: TaskStatus | null
  site: string | null
  search: string
  sort: { id: string; desc: boolean } | null
}

export const DEFAULT_FILTERS: AuditFilters = {
  agentId: null,
  status: null,
  site: null,
  search: '',
  sort: null,
}

const KEYS = {
  agent: 'agent',
  status: 'status',
  site: 'site',
  q: 'q',
  sort: 'sort',
} as const

const VALID_STATUS = new Set<TaskStatus>(['live', 'done', 'failed'])

export function paramsToFilters(params: URLSearchParams): AuditFilters {
  const statusRaw = params.get(KEYS.status)
  const status = VALID_STATUS.has(statusRaw as TaskStatus)
    ? (statusRaw as TaskStatus)
    : null
  const sortRaw = params.get(KEYS.sort)
  let sort: AuditFilters['sort'] = null
  if (sortRaw) {
    const [id, dir] = sortRaw.split(':')
    if (id) sort = { id, desc: dir !== 'asc' }
  }
  return {
    agentId: params.get(KEYS.agent),
    status,
    site: params.get(KEYS.site),
    search: params.get(KEYS.q) ?? '',
    sort,
  }
}

export function filtersToParams(filters: AuditFilters): URLSearchParams {
  const params = new URLSearchParams()
  if (filters.agentId) params.set(KEYS.agent, filters.agentId)
  if (filters.status) params.set(KEYS.status, filters.status)
  if (filters.site) params.set(KEYS.site, filters.site)
  if (filters.search) params.set(KEYS.q, filters.search)
  if (filters.sort) {
    params.set(
      KEYS.sort,
      `${filters.sort.id}:${filters.sort.desc ? 'desc' : 'asc'}`,
    )
  }
  return params
}
