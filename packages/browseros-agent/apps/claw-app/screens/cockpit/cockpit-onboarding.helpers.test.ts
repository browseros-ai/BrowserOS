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
  TRY_PROMPTS,
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
    expect(
      [
        FIRST_RUN_COPY.ready,
        FIRST_RUN_COPY.lineAccent,
        FIRST_RUN_COPY.lineTail,
      ].join(' '),
    ).toBe('You’re all set. Watch me do one.')
    expect(FIRST_RUN_COPY.noApps).toBe('Almost there.')
    expect(
      `${FIRST_RUN_COPY.coworkQuestion} ${FIRST_RUN_COPY.coworkAction}`,
    ).toBe('Using Claude Cowork? Set it up')
    expect(FIRST_RUN_COPY.tryHint).toBe(
      'Paste one into Claude Code or Codex, then watch it here.',
    )
    const copy = [...Object.values(FIRST_RUN_COPY), ...TRY_PROMPTS].join(' ')
    expect(copy).not.toMatch(/\b(MCP|server|endpoint|terminal|config)\b/i)
  })

  it('offers the demo’s own chore first, then an everyday one', () => {
    expect(TRY_PROMPTS).toEqual([
      'Find the cheapest flight to New York next Friday.',
      'Read me today’s top news.',
    ])
  })

  it('sends people without an AI app to the Claude download', () => {
    expect(GET_CLAUDE_URL).toBe('https://claude.ai/download')
  })
})
