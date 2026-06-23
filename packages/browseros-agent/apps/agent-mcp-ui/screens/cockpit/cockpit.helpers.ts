import type { ActivityRow } from '@/modules/api/activity.hooks'
import type { AgentRow } from '@/modules/api/agents.hooks'
import type { TabActivityRecord } from '@/modules/api/tabs.hooks'

// Small palette so each agent gets a stable colour without joining a
// profile lookup. Hash the slug; if two agents collide it is purely
// cosmetic.
const PALETTE = [
  '#F26B2A',
  '#2F6FE0',
  '#7A5AF8',
  '#10A37F',
  '#E0561C',
  '#0EA5E9',
  '#F59E0B',
  '#DB2777',
]

export function colorForSlug(slug: string): string {
  let hash = 0
  for (let i = 0; i < slug.length; i++) {
    hash = (hash * 31 + slug.charCodeAt(i)) >>> 0
  }
  return PALETTE[hash % PALETTE.length] ?? PALETTE[0]
}

export function siteOf(url: string): string {
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return url
  }
}

export function formatRelative(ms: number, now: number): string {
  const delta = Math.max(0, now - ms)
  const seconds = Math.floor(delta / 1000)
  if (seconds < 60) return `${seconds}s ago`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m ago`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h ago`
  const days = Math.floor(hours / 24)
  return `${days}d ago`
}

/**
 * Tabs whose `status === 'active'` become live agent cards. The
 * label, harness, and color are derived locally from the slug since
 * PR 1 does not join the agent profile.
 */
export function tabsToAgentRows(records: TabActivityRecord[]): AgentRow[] {
  return records
    .filter((r) => r.status === 'active')
    .map((r) => ({
      id: r.targetId,
      label: r.slug,
      // TODO(pr-3 homepage): join the agent profile and surface the
      // real harness; today every row reads "Claude Code" because the
      // TabActivityRecord does not carry it.
      harness: 'Claude Code',
      site: siteOf(r.url),
      task: r.title || siteOf(r.url),
      status: 'running' as const,
      liveLine: `${r.lastToolName} - ${r.title || siteOf(r.url)}`,
      color: colorForSlug(r.slug),
    }))
}

/**
 * Idle records flow into RecentActivity so the user can see the last
 * thing each agent did on a tab even after the active window expires.
 */
export function tabsToActivityRows(
  records: TabActivityRecord[],
  now: number,
): ActivityRow[] {
  return records
    .filter((r) => r.status === 'idle')
    .map((r) => ({
      id: r.targetId,
      agentLabel: r.slug,
      color: colorForSlug(r.slug),
      status: 'done' as const,
      action: `${r.lastToolName} on ${r.title || siteOf(r.url)}`,
      site: siteOf(r.url),
      when: formatRelative(r.lastToolAt, now),
    }))
}
