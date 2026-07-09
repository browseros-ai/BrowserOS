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
  eyebrow: 'GET STARTED',
  h1Prefix: 'You watch. Your agent',
  h1Accent: 'works.',
  subhead:
    'Set up BrowserClaw once. Tell your AI to use it. Everything lands here.',
} as const

export const PRIMARY_ACTION_COPY = {
  install: {
    activeLabel: 'Set up MCP endpoint',
    doneLabel: 'View MCP endpoint',
    href: '/mcp',
  },
} as const

export const WAITING_COPY = {
  connectedNoActivity:
    'Waiting for your first run. Come back here as soon as you press enter in your agent.',
  promptCopied:
    'Prompt copied. Paste it into Claude Code, Cursor, or Codex, then press enter.',
} as const

export const STARTER_PROMPT_LABEL =
  'Paste this into Claude Code, Cursor, or Codex.'

export const STARTER_PROMPT =
  'Use BrowserClaw. Book me the cheapest morning flight from SFO to NYC next Friday.'

export const STEP_COPY = {
  install: {
    activeTitle: 'Install BrowserClaw as an MCP.',
    doneTitle: 'MCP installed.',
  },
  ask: {
    title: 'Prompt your agent.',
  },
  watch: {
    title: 'Watch it here.',
  },
} as const

export const FOOTER_COPY = {
  docs: 'Read the docs',
  // Deep-link to the BrowserClaw section instead of the docs root
  // so a first-run reader lands on install / first-run / MCP setup
  // instead of BrowserOS's general index.
  docsHref: 'https://docs.browseros.com/browserclaw/',
} as const
