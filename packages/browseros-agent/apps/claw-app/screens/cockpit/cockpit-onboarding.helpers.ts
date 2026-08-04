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

export const HERO_COPY = {
  eyebrow: 'WELCOME',
  h1Prefix: 'You watch. Your agent',
  h1Accent: 'works.',
  subhead:
    'Your agents are wired in. Watch a quick demo, then hand your first task to any of them.',
} as const

export const PANEL_COPY = {
  heading: 'Hand off your first task',
  // Three status messages share one fixed-height slot so copying never
  // reflows the panel. Keep each to at most two lines at panel width.
  tieBack: 'Then come back here to watch it run.',
  waiting: 'Waiting for your first run. Come back the moment you press enter.',
  copied: 'Copied. Paste it into your agent, then watch it here.',
} as const

export const MANAGE_COPY = {
  label: 'Manage agents',
  href: '/mcp',
} as const

export const STARTER_PROMPT_LABEL = 'Paste this prompt into your agent.'

export const STARTER_PROMPT =
  'Using BrowserOS neo, search for the current monthly prices of streaming services such as Netflix, Disney plus, Hulu, Max and Apple TV'

export const CONNECTED_COPY = {
  suffix: 'connected',
} as const

export const FOOTER_COPY = {
  docs: 'Read the docs',
  // Deep-link to the BrowserClaw section instead of the docs root
  // so a first-run reader lands on install / first-run / MCP setup
  // instead of BrowserOS's general index. Mintlify canonicalises
  // the URL without a trailing slash, so we use the no-slash form
  // to avoid triggering a redirect on every click.
  docsHref: 'https://docs.browseros.com/browserclaw',
} as const
