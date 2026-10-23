/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  FIRST_RUN_COPY,
  GET_CLAUDE_URL,
  getOnboardingState,
  TRY_PROMPT,
} from './cockpit-onboarding.helpers'

describe('getOnboardingState', () => {
  it('returns first-run when no connection and no activity', () => {
    expect(
      getOnboardingState({ hasConnection: false, hasActivity: false }),
    ).toBe('first-run')
  })

  it('returns waiting when connection is installed but no activity yet', () => {
    expect(
      getOnboardingState({ hasConnection: true, hasActivity: false }),
    ).toBe('waiting')
  })

  it('returns ready as soon as any activity exists, regardless of connection state', () => {
    expect(
      getOnboardingState({ hasConnection: false, hasActivity: true }),
    ).toBe('ready')
    expect(getOnboardingState({ hasConnection: true, hasActivity: true })).toBe(
      'ready',
    )
  })
})

describe('cockpit first-run copy', () => {
  // Pins the shipped copy so accidental voice drift is caught. Every string
  // change here is a copy-pass decision, not a mechanical fix.
  it('neo speaks once, in plain words', () => {
    const line = ({
      lead,
      accent,
      tail,
    }: {
      lead: string | null
      accent: string
      tail: string
    }) => [lead, accent, tail].filter(Boolean).join(' ')
    expect(line(FIRST_RUN_COPY.ready)).toBe('Watch me, then you try.')
    expect(line(FIRST_RUN_COPY.noApps)).toBe('Almost there. Watch me first.')
    expect(
      `${FIRST_RUN_COPY.coworkQuestion} ${FIRST_RUN_COPY.coworkAction}`,
    ).toBe('Using Claude Cowork? Set it up')
    expect(FIRST_RUN_COPY.tryLabel).toBe('Your turn')
    expect(FIRST_RUN_COPY.restartHint).toBe('If one is open, restart it once.')
    const copy = [
      ...Object.values(FIRST_RUN_COPY).flatMap((value) =>
        typeof value === 'string' ? [value] : Object.values(value),
      ),
      TRY_PROMPT,
    ].join(' ')
    expect(copy).not.toMatch(/\b(MCP|server|endpoint|terminal|config)\b/i)
  })

  it('offers one task: the demo’s own, word for word', () => {
    expect(TRY_PROMPT).toBe(
      'Use neo, find me the cheapest one-way flight to NYC next Friday on Google Flights.',
    )
  })

  it('sends people without an AI app to the Claude download', () => {
    expect(GET_CLAUDE_URL).toBe('https://claude.ai/download')
  })
})
