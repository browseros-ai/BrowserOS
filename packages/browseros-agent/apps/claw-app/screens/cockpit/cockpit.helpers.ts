import type {
  HelpRequest,
  LiveSessionActivityState,
  SessionBrowserTab,
  SessionSummary,
  ToolEvent,
} from '@browseros/claw-api'
import { agentIdentity } from '@/components/harness/harness.types'

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

export interface LiveSessionCardRecord {
  sessionId: string
  profileId?: string
  slug: string
  label: string
  name: string
  /** The connected agent's harness name, such as "Claude Code", when known. */
  harness?: string
  /** The agent's name for sentences, such as "Claude" or "Codex". */
  agentName: string
  color: string
  startedAt: number
  state: LiveSessionActivityState
  selectedTab: SessionBrowserTab | null
  browserTabs: SessionBrowserTab[]
  toolCount: number
  recentTools: ToolEvent[]
  /** Present only while the agent is parked waiting for a human on this session. */
  helpRequest?: HelpRequest
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
      harness: session.harness,
      agentName: agentIdentity(session.harness, session.label || session.slug)
        .name,
      color: session.color ?? colorForSlug(session.slug),
      startedAt: session.startedAt,
      state: session.live?.state ?? 'idle',
      selectedTab,
      browserTabs,
      toolCount: browserTabs.reduce((sum, tab) => sum + tab.toolCount, 0),
      recentTools,
      helpRequest: session.live?.helpRequest,
    }
  })

  return cards.sort(
    (left, right) =>
      left.startedAt - right.startedAt ||
      left.sessionId.localeCompare(right.sessionId),
  )
}

export type PartOfDay = 'morning' | 'afternoon' | 'evening'

/** Morning from 5:00 to 11:59, afternoon to 16:59, evening otherwise. */
export function partOfDay(hour: number): PartOfDay {
  if (hour >= 5 && hour < 12) return 'morning'
  if (hour >= 12 && hour < 17) return 'afternoon'
  return 'evening'
}

export interface TimeSaved {
  amount: number
  unit: 'hour' | 'minute'
}

const MINUTE_MS = 60_000
const HOUR_MS = 60 * MINUTE_MS

/** Rounds measured time saved to whole hours, or minutes under an hour; null under a minute. */
export function roundTimeSaved(ms: number): TimeSaved | null {
  if (ms >= HOUR_MS) return { amount: Math.round(ms / HOUR_MS), unit: 'hour' }
  const minutes = Math.round(ms / MINUTE_MS)
  return minutes >= 1 ? { amount: minutes, unit: 'minute' } : null
}

/** The line under the greeting: what the agents are doing in one sentence. */
export function homeLede(
  sessions: LiveSessionCardRecord[],
  inControl: LiveSessionCardRecord | null,
): string {
  if (inControl) return `You’re helping ${inControl.agentName} with one task.`
  const waiting = sessions.filter((session) => session.helpRequest)
  if (waiting.length === 1) {
    return `${waiting[0]?.agentName} needs your help with one task.`
  }
  if (waiting.length > 1) return `${waiting.length} tasks need your help.`
  if (sessions.length === 1) return 'Your AI agents are working on one task.'
  if (sessions.length > 1) {
    return `Your AI agents are working on ${sessions.length} tasks.`
  }
  return 'Nothing is running right now.'
}

/** The site a help request is about, in the words a person would use. */
export function helpRequestSite(help: HelpRequest): string {
  if (help.url) return siteOf(help.url)
  return help.title || 'this'
}

/** "Claude Code needs you to sign in to linkedin.com", from the request's kind. */
export function helpRequestTitle(app: string, help: HelpRequest): string {
  const site = help.url ? siteOf(help.url) : help.title || 'a page'
  switch (help.kind) {
    case 'login':
      return `${app} needs you to sign in to ${site}`
    case 'captcha':
      return `${app} needs you to prove you’re not a robot on ${site}`
    case 'approval':
      return `${app} wants your OK on ${site}`
    default:
      return `${app} needs a hand on ${site}`
  }
}

/** "Waiting 3 min", or "Just asked" in the first minute. */
export function waitingLabel(requestedAt: number, now: number): string {
  const minutes = Math.floor(Math.max(0, now - requestedAt) / MINUTE_MS)
  return minutes < 1 ? 'Just asked' : `Waiting ${minutes} min`
}
