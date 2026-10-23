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
 * neo's one line on the cockpit's first run (final Paper, Try one on top).
 * The onboarding page ends after the import, so this is where neo picks the
 * conversation back up: the apps it connected (the cockpit has the real list;
 * the page did not), the one prompt to try, the demo of it, and the Claude
 * Cowork step. Plain words only: no MCP, server or config.
 */
export const FIRST_RUN_COPY = {
  /** With apps connected: "*Watch me,* then you try." */
  ready: { lead: null, accent: 'Watch me,', tail: 'then you try.' },
  /** With none found: "Almost there. *Watch me* first." */
  noApps: { lead: 'Almost there.', accent: 'Watch me', tail: 'first.' },
  connected: 'Connected',
  connectedApps: 'Connected apps',
  /** The popover's footer: a running app only sees neo after a restart. */
  restartHint: 'If one is open, restart it once.',
  noneFound: 'I didn’t find an AI app yet.',
  getClaude: 'Get Claude',
  coworkQuestion: 'Using Claude Cowork?',
  coworkAction: 'Set it up',
  tryLabel: 'Your turn',
} as const

export const GET_CLAUDE_URL = 'https://claude.ai/download'

/**
 * The one task to try (owner). The demo types these exact words in every app,
 * so the pill copies what the person just watched.
 */
export const TRY_PROMPT =
  'Use neo, find me the cheapest one-way flight to NYC next Friday on Google Flights.'
