/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { getOnboardingState } from './cockpit-onboarding.helpers'

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
