/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Messages between the in-page takeover bar (content script) and the background.
 * The content script never talks to the claw server directly: a page-origin fetch
 * would be a cross-origin request, so the background (which holds host
 * permissions) does the lookups and the resolve.
 */

/** What the bar needs to render, for the tab the message is answered on. */
export interface TakeoverContext {
  sessionId: string
  agentLabel: string
  reason: string
  resumeHint?: string
  site?: string
  /** Epoch ms the request opened, for the waiting timer. */
  requestedAt: number
}

/**
 * Background's answer to a `takeover-poll`, resolved from the live help request
 * for the sender's tab.
 */
export type TakeoverPollResponse =
  | { active: true; context: TakeoverContext }
  | { active: false }

/** A `takeover-resolve`: the human handed control back from the bar. */
export interface TakeoverResolveMessage {
  type: 'takeover-resolve'
  sessionId: string
  note?: string
}
