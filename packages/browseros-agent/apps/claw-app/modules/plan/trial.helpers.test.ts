/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import type { Plan, PlanPricing } from '@browseros/claw-api'
import {
  isCardTrial,
  monthlyTrialDays,
  trialDaysLeft,
  trialOfferDays,
} from './trial.helpers'

const DAY = 86_400_000
const NOW = Date.UTC(2026, 10, 25, 12)

const pricingWith = (monthTrial?: number): PlanPricing => ({
  plans: [
    {
      plan: 'ultrafast',
      name: 'ultrafast',
      description: null,
      features: ['claw.ultrafast'],
      prices: [
        {
          interval: 'month',
          intervalCount: 1,
          amount: 2000,
          currency: 'usd',
          trialDays: monthTrial,
        },
        {
          interval: 'year',
          intervalCount: 1,
          amount: 20000,
          currency: 'usd',
          trialDays: 0,
        },
      ],
    },
  ],
})

const planOf = (overrides: Partial<Plan> = {}): Plan => ({
  available: true,
  plan: null,
  planName: null,
  features: [],
  status: 'none',
  periodEnd: null,
  endsAt: null,
  graceUntil: null,
  onKey: false,
  checkedAt: null,
  stale: false,
  ...overrides,
})

describe('the free trial offer', () => {
  it('tests that it comes from the monthly price only', () => {
    expect(monthlyTrialDays(pricingWith(7))).toBe(7)
    expect(monthlyTrialDays(pricingWith(undefined))).toBe(0)
    expect(monthlyTrialDays(undefined)).toBe(0)
  })

  it('tests that only Free installs never on a key are offered one', () => {
    expect(trialOfferDays(planOf(), pricingWith(7))).toBe(7)
    expect(
      trialOfferDays(planOf({ onKey: true }), pricingWith(7)),
      'a lapsed key gets Renew, not a trial',
    ).toBe(0)
    expect(trialOfferDays(planOf({ status: 'active' }), pricingWith(7))).toBe(0)
    expect(trialOfferDays(planOf({ available: false }), pricingWith(7))).toBe(0)
    expect(trialOfferDays(planOf(), pricingWith(0))).toBe(0)
  })
})

describe('a running trial', () => {
  it('tests that only a card trial is shown as a trial', () => {
    expect(isCardTrial({ trial: true, trialKind: 'card' })).toBe(true)
    expect(isCardTrial({ trial: true, trialKind: 'grant' })).toBe(false)
    expect(isCardTrial({ trial: true })).toBe(false)
    expect(isCardTrial({ trial: false, trialKind: 'card' })).toBe(false)
  })

  it('tests that days left count whole days, rounded up', () => {
    const trial = (endsIn: number) => ({
      trial: true,
      trialKind: 'card' as const,
      trialEndsAt: NOW + endsIn,
    })
    expect(trialDaysLeft(trial(7 * DAY), NOW)).toBe(7)
    expect(trialDaysLeft(trial(2 * DAY + 1), NOW)).toBe(3)
    expect(trialDaysLeft(trial(-DAY), NOW)).toBe(0)
    expect(
      trialDaysLeft(
        { trial: true, trialKind: 'grant', trialEndsAt: NOW + DAY },
        NOW,
      ),
    ).toBeNull()
  })
})
