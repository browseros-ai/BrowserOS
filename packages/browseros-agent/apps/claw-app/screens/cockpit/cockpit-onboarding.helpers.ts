/**
 * @license
 * Copyright 2025 BrowserOS
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

/** Status labels for the three step badges, per state. */
export const STEP_KICKERS = {
  'first-run': ['Next up', 'Then', 'Finally'] as const,
  waiting: ['Done', 'Now', 'Next'] as const,
} as const

export const HERO_COPY = {
  eyebrow: 'Get started',
  h1Prefix: "Let's get your first agent",
  h1Accent: 'running.',
  subhead: 'Three steps. About two minutes.',
} as const

export const STEP_COPY = {
  install: {
    title: 'Install the MCP.',
    body: 'One endpoint that connects BrowserClaw to Claude Code, Cursor, Codex, and every harness that speaks MCP.',
    doneTitle: 'MCP installed.',
    doneBody:
      'Your endpoint is live. Any agent on your list can now open a browser session.',
    cta: 'Set up',
    doneCta: 'View endpoints',
    href: '/mcp',
  },
  ask: {
    title: 'Ask your agent to try it.',
    body: 'Any tool that speaks MCP works. Claude Code, Cursor, Codex. Kick off a task on the web.',
    terminal: [
      '$ claude',
      '> Try BrowserClaw. Book me',
      '  the cheapest flight to',
      '  London for next weekend.',
      '> opening BrowserClaw...',
    ] as const,
  },
  watch: {
    title: 'Watch it happen right here.',
    body: 'Every tool call, screenshot, and result shows up on this page as your agent works. Rewind any session from the audit view.',
  },
} as const

export const FOOTER_COPY = {
  refreshQuestion: 'Already set up?',
  refresh: 'Refresh the page.',
  docs: 'Read the docs',
  docsHref: 'https://docs.browseros.com/',
} as const
