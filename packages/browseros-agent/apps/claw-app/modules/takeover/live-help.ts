/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { SessionSummary } from '@browseros/claw-api'
import type { TakeoverContext } from './takeover.types'

function hostOf(url: string | undefined): string | undefined {
  if (!url) return undefined
  try {
    return new URL(url).hostname.replace(/^www\./, '')
  } catch {
    return undefined
  }
}

/**
 * The takeover context for a Chrome tab, or null when no live session is parked
 * on that exact tab. Pairs with the tab pinned on the help request at request
 * time, so the bar lands on the blocked page even when the agent has many tabs.
 */
export function takeoverForTab(
  sessions: SessionSummary[],
  tabId: number,
): TakeoverContext | null {
  for (const session of sessions) {
    const help = session.live?.helpRequest
    if (help && help.browserTabId === tabId) {
      return {
        sessionId: session.sessionId,
        agentLabel: session.label || session.slug,
        reason: help.reason,
        resumeHint: help.resumeHint,
        site: hostOf(help.url) ?? help.title,
        requestedAt: help.requestedAt,
      }
    }
  }
  return null
}
