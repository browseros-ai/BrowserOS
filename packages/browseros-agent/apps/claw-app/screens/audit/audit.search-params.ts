/** "All", or only the tasks that need a look: ones that failed or were stopped. */
export type HistoryView = 'all' | 'attention'

export interface AuditFilters {
  agentSlug: string | null
  view: HistoryView
  search: string
}

export const DEFAULT_FILTERS: AuditFilters = {
  agentSlug: null,
  view: 'all',
  search: '',
}

const KEYS = {
  agent: 'agent',
  view: 'view',
  q: 'q',
} as const

export function paramsToFilters(params: URLSearchParams): AuditFilters {
  return {
    agentSlug: params.get(KEYS.agent),
    view: params.get(KEYS.view) === 'attention' ? 'attention' : 'all',
    search: params.get(KEYS.q) ?? '',
  }
}

export function filtersToParams(filters: AuditFilters): URLSearchParams {
  const params = new URLSearchParams()
  if (filters.agentSlug) params.set(KEYS.agent, filters.agentSlug)
  if (filters.view === 'attention') params.set(KEYS.view, 'attention')
  if (filters.search) params.set(KEYS.q, filters.search)
  return params
}
