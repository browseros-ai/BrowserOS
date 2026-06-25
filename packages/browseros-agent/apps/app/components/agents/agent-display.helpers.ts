import type { AgentListItem } from '@/modules/agents/agents-page-types'
import type { AgentLiveness } from './LivenessDot'

/**
 * Display rules for the redesigned agent rows. Pure helpers — no React,
 * no API calls — so they're trivial to unit-test and the row card stays
 * focused on layout.
 */

const UUID_PATTERN =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i

/**
 * Prefer the explicit name when it differs meaningfully from the id;
 * otherwise fall back to a short id prefix.
 */
export function displayName(agent: AgentListItem): string {
  const name = agent.name?.trim()
  const id = agent.agentId
  if (!name || name === id) {
    if (UUID_PATTERN.test(id)) return id.slice(0, 8)
    return id
  }
  return name
}

export function canDelete(agent: AgentListItem): boolean {
  return agent.canDelete
}

const ONE_MINUTE = 60_000
const ONE_HOUR = 60 * ONE_MINUTE
const ONE_DAY = 24 * ONE_HOUR

/**
 * Lightweight relative-time formatter. We don't want to drag in
 * `dayjs/relativeTime` just for a few labels.
 */
export function formatRelativeTime(epochMs: number | null): string {
  if (epochMs === null || !Number.isFinite(epochMs)) return 'never'
  const diff = Math.max(0, Date.now() - epochMs)
  if (diff < ONE_MINUTE) return 'just now'
  if (diff < ONE_HOUR) {
    const m = Math.floor(diff / ONE_MINUTE)
    return `${m} min ago`
  }
  if (diff < ONE_DAY) {
    const h = Math.floor(diff / ONE_HOUR)
    return h === 1 ? '1 hr ago' : `${h} hr ago`
  }
  const d = Math.floor(diff / ONE_DAY)
  return d === 1 ? '1 day ago' : `${d} days ago`
}

/**
 * Tooltip-friendly description of a row's current liveness state.
 * Returns `undefined` when the state has nothing extra to add (e.g.
 * `unknown` with no timestamp).
 */
export function livenessDetail(
  status: AgentLiveness,
  lastUsedAt: number | null | undefined,
): string | undefined {
  if (lastUsedAt == null) return undefined
  const diffMin = Math.floor((Date.now() - lastUsedAt) / 60_000)
  if (status === 'idle') return `Idle for ${Math.max(0, diffMin)} min`
  if (status === 'asleep') {
    if (diffMin < 60) return `Asleep — quiet for ${diffMin} min`
    const hr = Math.floor(diffMin / 60)
    return `Asleep — quiet for ${hr} hr`
  }
  if (status === 'working') return 'Working on a turn'
  if (status === 'error') return 'Attention — last turn failed'
  return undefined
}
