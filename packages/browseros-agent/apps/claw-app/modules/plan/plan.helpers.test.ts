/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import {
  hasUltrafast,
  planContextFor,
  showsUltrafastNudge,
} from './plan.helpers'

describe('the plan', () => {
  it('tests that ultrafast comes from the plan features', () => {
    expect(hasUltrafast({ features: ['claw.ultrafast'] })).toBe(true)
    expect(hasUltrafast({ features: [] })).toBe(false)
    expect(hasUltrafast(undefined)).toBe(false)
  })

  it('tests that only a free plan where ultrafast is on sale hears about it', () => {
    expect(
      showsUltrafastNudge({ available: true, features: ['claw.ultrafast'] }),
    ).toBe(false)
    expect(showsUltrafastNudge({ available: true, features: [] })).toBe(true)
    expect(showsUltrafastNudge({ available: false, features: [] })).toBe(false)
    expect(showsUltrafastNudge(undefined)).toBe(false)
  })
})

describe('the plan analytics events carry', () => {
  it('tests that events name the plan, and say trial only when the plan does', () => {
    expect(planContextFor(undefined)).toBeNull()
    expect(planContextFor({ features: [] })).toEqual({ plan: 'free' })
    expect(planContextFor({ features: ['claw.ultrafast'] })).toEqual({
      plan: 'ultrafast',
    })
    expect(
      planContextFor({ features: ['claw.ultrafast'], trial: true }),
    ).toEqual({ plan: 'ultrafast', trial: true })
  })
})
