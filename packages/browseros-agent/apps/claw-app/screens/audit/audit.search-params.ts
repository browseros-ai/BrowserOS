import type { TaskStatus } from '@/modules/api/audit.hooks'

export interface AuditFilters {
  agentSlug: string | null
  status: TaskStatus | null
  site: string | null
  search: string
}

export const DEFAULT_FILTERS: AuditFilters = {
  agentSlug: null,
  status: null,
  site: null,
  search: '',
}

const KEYS = {
  agent: 'agent',
  status: 'status',
  site: 'site',
  q: 'q',
} as const

const VALID_STATUS = new Set<TaskStatus>([
  'live',
  'done',
  'failed',
  'cancelled',
])

export function paramsToFilters(params: URLSearchParams): AuditFilters {
  const statusRaw = params.get(KEYS.status)
  const status = VALID_STATUS.has(statusRaw as TaskStatus)
    ? (statusRaw as TaskStatus)
    : null
  return {
    agentSlug: params.get(KEYS.agent),
    status,
    site: params.get(KEYS.site),
    search: params.get(KEYS.q) ?? '',
  }
}

export function filtersToParams(filters: AuditFilters): URLSearchParams {
  const params = new URLSearchParams()
  if (filters.agentSlug) params.set(KEYS.agent, filters.agentSlug)
  if (filters.status) params.set(KEYS.status, filters.status)
  if (filters.site) params.set(KEYS.site, filters.site)
  if (filters.search) params.set(KEYS.q, filters.search)
  return params
}
