import type {
  LiveSessionActivityState,
  SessionBrowserTab,
  SessionSummary,
  ToolEvent,
} from '@browseros/claw-api'
import { HARNESSES, type Harness } from '@/components/harness/harness.types'

// Missing parent colors fall back to a stable slug hash so card identity does
// not flicker between live-session polls.
const PALETTE = [
  '#16A34A',
  '#2F6FE0',
  '#7A5AF8',
  '#10A37F',
  '#EA580C',
  '#0EA5E9',
  '#F59E0B',
  '#DB2777',
]

const TRAIL_DISPLAY_CAP = 4
const MERGED_TRAIL_CAP = 8

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

/** Returns the most recent N tool names joined for compact card trail rows. */
export function formatToolTrail(
  recentTools: ToolEvent[],
  max: number = TRAIL_DISPLAY_CAP,
): string {
  if (recentTools.length === 0) return ''
  return recentTools
    .slice(-max)
    .map((tool) => tool.name)
    .join(' -> ')
}

/** Coerces contract strings into the UI harness union with an honest fallback. */
export function harnessForRow(value: string | undefined): Harness {
  if (!value) return 'Claude Code'
  return (HARNESSES as readonly string[]).includes(value)
    ? (value as Harness)
    : 'Claude Code'
}

export interface LiveSessionCardRecord {
  sessionId: string
  profileId?: string
  slug: string
  label: string
  name: string
  harness: Harness
  color: string
  startedAt: number
  state: LiveSessionActivityState
  selectedTab: SessionBrowserTab | null
  browserTabs: SessionBrowserTab[]
  toolCount: number
  recentTools: ToolEvent[]
}

export interface LiveSessionCardOptions {
  /** Previous selection keyed by session id and carrying a Chrome tab id. */
  stickySelection?: ReadonlyMap<string, number>
}

function activityAt(tab: SessionBrowserTab): number | undefined {
  return tab.lastActivityAt ?? tab.firstActivityAt
}

function compareBrowserTabs(
  left: SessionBrowserTab,
  right: SessionBrowserTab,
): number {
  const leftActivity = activityAt(left)
  const rightActivity = activityAt(right)
  if (leftActivity === undefined && rightActivity !== undefined) return 1
  if (leftActivity !== undefined && rightActivity === undefined) return -1
  if (leftActivity !== undefined && rightActivity !== undefined) {
    const byActivity = rightActivity - leftActivity
    if (byActivity !== 0) return byActivity
  }
  return left.browserTabId - right.browserTabId
}

/**
 * Projects each connected session into one card. A previous browser-tab id
 * remains selected while owned; otherwise activity freshness and then Chrome
 * tab id provide deterministic election. Card order follows session arrival.
 */
export function sessionsToLiveCards(
  sessions: SessionSummary[],
  options?: LiveSessionCardOptions,
): LiveSessionCardRecord[] {
  const cards = sessions.map((session) => {
    const browserTabs = [...(session.live?.browserTabs ?? [])].sort(
      compareBrowserTabs,
    )
    const previousSelection = options?.stickySelection?.get(session.sessionId)
    const selectedTab =
      browserTabs.find((tab) => tab.browserTabId === previousSelection) ??
      browserTabs[0] ??
      null
    const recentTools = browserTabs
      .flatMap((tab) => tab.recentTools)
      .sort((left, right) => left.at - right.at)
      .slice(-MERGED_TRAIL_CAP)

    return {
      sessionId: session.sessionId,
      profileId: session.profileId,
      slug: session.slug,
      label: session.label || session.slug,
      name: session.name,
      harness: harnessForRow(session.harness),
      color: session.color ?? colorForSlug(session.slug),
      startedAt: session.startedAt,
      state: session.live?.state ?? 'idle',
      selectedTab,
      browserTabs,
      toolCount: browserTabs.reduce((sum, tab) => sum + tab.toolCount, 0),
      recentTools,
    }
  })

  return cards.sort(
    (left, right) =>
      left.startedAt - right.startedAt ||
      left.sessionId.localeCompare(right.sessionId),
  )
}
