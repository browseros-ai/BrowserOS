/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure helpers for the cockpit onboarding block. The Cockpit screen
 * reads live query state, feeds the two derived booleans through
 * `getOnboardingState()`, and renders the returned discriminant.
 *
 * Keeping the selector pure means every state variant is trivially
 * unit-testable without React. The onboarding component consumes the
 * discriminant and the copy constants; it never re-derives state.
 */

export type OnboardingState = 'first-run' | 'waiting' | 'ready'

export interface OnboardingSignals {
  /** True when at least one MCP connection is installed. */
  hasConnection: boolean
  /** True when the recent-activity list has at least one task row. */
  hasActivity: boolean
}

/**
 * Discriminant for the cockpit view. `ready` means the reader has
 * already completed the loop at least once, so the normal cockpit
 * renders unchanged.
 */
export function getOnboardingState({
  hasConnection,
  hasActivity,
}: OnboardingSignals): OnboardingState {
  if (hasActivity) return 'ready'
  if (hasConnection) return 'waiting'
  return 'first-run'
}

/**
 * neo's one line on the cockpit's first run (round 7). The onboarding page
 * ends after the import, so this is where neo picks the conversation back up:
 * the apps it connected (the cockpit has the real list; the page did not), the
 * Claude Cowork step, then the demo. Plain words only: no MCP, server or config.
 */
export const FIRST_RUN_COPY = {
  /** With apps connected; "Almost there." when none were found. */
  ready: 'You’re all set.',
  noApps: 'Almost there.',
  lineAccent: 'Watch me',
  lineTail: 'do one.',
  connected: 'Connected',
  connectedApps: 'Connected apps',
  restartHint: 'If one is open, restart it once.',
  noneFound: 'I didn’t find an AI app yet.',
  getClaude: 'Get Claude',
  coworkQuestion: 'Using Claude Cowork?',
  coworkAction: 'Set it up',
  tryLabel: 'Try one',
  tryHint: 'Paste one into Claude or ChatGPT, then watch it here.',
} as const

export const GET_CLAUDE_URL = 'https://claude.ai/download'

/** The same chore the demo shows first, then an everyday one. */
export const TRY_PROMPTS = [
  'Find the cheapest flight to New York next Friday.',
  'Read me today’s top news.',
] as const
