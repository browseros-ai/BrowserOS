import type { TaskStatus, TaskSummary } from '@/modules/api/audit.hooks'

const NINE_SECONDS = 9_000
const ONE_MINUTE = 60_000
const ONE_HOUR = 3_600_000
const ONE_DAY = 86_400_000

export function formatRelative(createdAt: number, now: number): string {
  const delta = now - createdAt
  if (delta < NINE_SECONDS) return 'just now'
  if (delta < ONE_MINUTE) return `${Math.floor(delta / 1000)}s ago`
  if (delta < ONE_HOUR) return `${Math.floor(delta / ONE_MINUTE)}m ago`
  if (delta < ONE_DAY) return `${Math.floor(delta / ONE_HOUR)}h ago`
  return `${Math.floor(delta / ONE_DAY)}d ago`
}

export function siteOf(url: string | null): string {
  if (!url) return ''
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}

export function formatDuration(ms: number): string {
  const v = ms < 0 ? 0 : ms
  if (v < 1000) return `${v}ms`
  const seconds = Math.floor(v / 1000)
  if (seconds < 60) return `${seconds}s`
  const mins = Math.floor(seconds / 60)
  const remSec = seconds % 60
  if (mins < 60) return `${mins}m ${remSec}s`
  const hours = Math.floor(mins / 60)
  const remMin = mins % 60
  return `${hours}h ${remMin}m`
}

/**
 * Short trail of tool names with an ellipsis when the sequence is
 * longer than `cap`. Mirrors the abbreviated trail shown on each
 * task card / row.
 */
export function abbreviateSequence(seq: string[], cap = 5): string {
  if (seq.length <= cap) return seq.join(' → ')
  return `${seq.slice(0, cap).join(' → ')} → …`
}

export interface AgentChip {
  agentId: string
  slug: string
  agentLabel: string
  count: number
}

export function agentChipsFor(tasks: TaskSummary[]): AgentChip[] {
  const map = new Map<string, AgentChip>()
  for (const t of tasks) {
    const existing = map.get(t.agentId)
    if (existing) {
      existing.count += 1
      continue
    }
    map.set(t.agentId, {
      agentId: t.agentId,
      slug: t.slug,
      agentLabel: t.agentLabel,
      count: 1,
    })
  }
  return [...map.values()].sort((a, b) => b.count - a.count)
}

export function statusOptions(
  tasks: TaskSummary[],
): { status: TaskStatus; count: number }[] {
  const counts: Record<TaskStatus, number> = { live: 0, done: 0, failed: 0 }
  for (const t of tasks) counts[t.status] += 1
  return (['live', 'done', 'failed'] as TaskStatus[])
    .filter((s) => counts[s] > 0)
    .map((s) => ({ status: s, count: counts[s] }))
}

export function siteOptions(
  tasks: TaskSummary[],
): { site: string; count: number }[] {
  const map = new Map<string, number>()
  for (const t of tasks) {
    if (!t.site) continue
    map.set(t.site, (map.get(t.site) ?? 0) + 1)
  }
  return [...map.entries()]
    .map(([site, count]) => ({ site, count }))
    .sort((a, b) => b.count - a.count)
}

export function parseResultMeta(raw: string | null): {
  isError: boolean
  contentSummary: string
  structuredKeys: string[]
} | null {
  if (!raw) return null
  try {
    const v = JSON.parse(raw) as {
      isError?: boolean
      contentSummary?: string
      structuredKeys?: string[]
    }
    return {
      isError: Boolean(v.isError),
      contentSummary: v.contentSummary ?? 'unknown',
      structuredKeys: Array.isArray(v.structuredKeys) ? v.structuredKeys : [],
    }
  } catch {
    return null
  }
}
