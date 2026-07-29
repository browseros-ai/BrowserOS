import type { LiveSessionCardRecord } from './cockpit.helpers'

export interface LeadRestSplit {
  lead: LiveSessionCardRecord | null
  rest: LiveSessionCardRecord[]
}

function lastToolAt(session: LiveSessionCardRecord): number {
  let latest = 0
  for (const tool of session.recentTools) {
    if (tool.at > latest) latest = tool.at
  }
  return latest
}

/**
 * Elects the session the operator most wants to watch as the lead: most
 * recent tool activity first, then most recent start, then session id so the
 * choice stays stable across live-session polls.
 */
export function pickLeadSession(
  sessions: LiveSessionCardRecord[],
): LeadRestSplit {
  if (sessions.length === 0) return { lead: null, rest: [] }
  const ranked = [...sessions].sort((left, right) => {
    const byTool = lastToolAt(right) - lastToolAt(left)
    if (byTool !== 0) return byTool
    const byStart = right.startedAt - left.startedAt
    if (byStart !== 0) return byStart
    return left.sessionId.localeCompare(right.sessionId)
  })
  const [lead, ...rest] = ranked
  return { lead: lead ?? null, rest }
}

/** Compact live-timer string without the trailing "ago" of formatRelative. */
export function formatElapsed(startedAt: number, now: number): string {
  const delta = Math.max(0, now - startedAt)
  const seconds = Math.floor(delta / 1000)
  if (seconds < 60) return `${seconds}s`
  const minutes = Math.floor(seconds / 60)
  if (minutes < 60) return `${minutes}m`
  const hours = Math.floor(minutes / 60)
  if (hours < 24) return `${hours}h`
  const days = Math.floor(hours / 24)
  return `${days}d`
}
