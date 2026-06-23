import type { ActivityRow } from '@/modules/api/activity.hooks'
import type { AgentRow } from '@/modules/api/agents.hooks'
import type { TabActivityRecord, ToolEvent } from '@/modules/api/tabs.hooks'
import { HARNESSES, type Harness } from '@/screens/new-agent/new-agent.schemas'

// Fallback palette used when the server-side join did not return a
// colour for the agent (today no profile field exists; the server
// always emits null). Hashing the slug keeps the colour stable per
// agent so cards do not flicker between polls.
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

const TRAIL_DISPLAY_CAP = 4

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
 * Returns the most recent N tool names joined with `->` for the
 * RunningCard's trail row. Capped to keep the card visually tight;
 * the full ring buffer is still in the data for any other consumer.
 */
export function formatToolTrail(
  recentTools: ToolEvent[],
  max: number = TRAIL_DISPLAY_CAP,
): string {
  if (recentTools.length === 0) return ''
  const tail = recentTools.slice(-max)
  return tail.map((t) => t.name).join(' -> ')
}

/**
 * Coerces the server-supplied harness string back into the UI's
 * `Harness` union. Unknown values (older profiles, hand-edited
 * files, server-side enum drift) fall back to 'Claude Code' so the
 * harness icon still resolves to something concrete.
 */
export function harnessForRow(value: string | null): Harness {
  if (!value) return 'Claude Code'
  return (HARNESSES as readonly string[]).includes(value)
    ? (value as Harness)
    : 'Claude Code'
}

/**
 * Tabs whose `status === 'active'` become live agent cards. The
 * label, harness, and colour all come from the route's profile join
 * when available, falling back to slug-derived defaults so a record
 * whose profile was deleted between dispatch and render still shows
 * something useful.
 */
export function tabsToAgentRows(records: TabActivityRecord[]): AgentRow[] {
  return records
    .filter((r) => r.status === 'active')
    .map((r) => ({
      id: r.targetId,
      label: r.agentLabel || r.slug,
      harness: harnessForRow(r.harness),
      site: siteOf(r.url),
      task: r.title || siteOf(r.url),
      status: 'running' as const,
      liveLine: `${r.lastToolName} - ${r.title || siteOf(r.url)}`,
      color: r.color ?? colorForSlug(r.slug),
      toolCount: r.toolCount,
      startedAt: r.firstToolAt,
      trail: formatToolTrail(r.recentTools),
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
      agentLabel: r.agentLabel || r.slug,
      color: r.color ?? colorForSlug(r.slug),
      status: 'done' as const,
      action: `${r.lastToolName} on ${r.title || siteOf(r.url)}`,
      site: siteOf(r.url),
      when: formatRelative(r.lastToolAt, now),
      toolCount: r.toolCount,
      trail: formatToolTrail(r.recentTools),
    }))
}
