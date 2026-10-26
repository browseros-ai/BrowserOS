/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure rules for raising a help request outside the page: which requests are
 * open, which are new or finished since the last look, and what to say.
 */

import type { SessionSummary } from '@browseros/claw-api'
import { agentIdentity } from '@/components/harness/harness.types'
import {
  helpRequestSite,
  helpRequestTitle,
} from '@/screens/cockpit/cockpit.helpers'

/** One help request an agent is waiting on, as the alerts need it. */
export interface OpenHelp {
  requestId: string
  sessionId: string
  browserTabId: number
  /** "Codex needs you to sign in to netflix.com" */
  title: string
  /** What to do, without the agent's own words, which can be private. */
  message: string
  requestedAt: number
}

/** Every open help request across the live sessions, oldest first. */
export function openHelpRequests(
  sessions: readonly SessionSummary[],
): OpenHelp[] {
  const open: OpenHelp[] = []
  for (const session of sessions) {
    const help = session.live?.helpRequest
    if (!help) continue
    const agent = agentIdentity(
      session.harness,
      session.label || session.slug,
    ).name
    open.push({
      requestId: help.requestId,
      sessionId: session.sessionId,
      browserTabId: help.browserTabId,
      title: helpRequestTitle(agent, help),
      message: `Open the ${helpRequestSite(help)} page, do what it needs, then hand control back.`,
      requestedAt: help.requestedAt,
    })
  }
  return open.sort((left, right) => left.requestedAt - right.requestedAt)
}

/** What changed since the request ids already announced. */
export function diffHelp(
  announced: readonly string[],
  open: readonly OpenHelp[],
): { started: OpenHelp[]; ended: string[] } {
  const openIds = new Set(open.map((help) => help.requestId))
  const seen = new Set(announced)
  return {
    started: open.filter((help) => !seen.has(help.requestId)),
    ended: announced.filter((requestId) => !openIds.has(requestId)),
  }
}

/** The toolbar badge for a count: empty at zero, "9+" past nine. */
export function badgeText(count: number): string {
  if (count <= 0) return ''
  return count > 9 ? '9+' : String(count)
}

/** The neo tab title while agents wait: "(1) Needs you · BrowserOS neo". */
export function attentionTitle(count: number, base: string): string {
  return count > 0 ? `(${count}) Needs you · ${base}` : base
}
