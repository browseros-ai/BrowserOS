/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { Plan } from '@browseros/claw-api'
import * as posthog from './posthog'

const captured: { event: string; properties?: Record<string, unknown> }[] = []
mock.module('./posthog', () => ({
  ...posthog,
  capture: (event: string, properties?: Record<string, unknown>) => {
    captured.push({ event, properties })
  },
}))

const storage: Record<string, string> = {}
Object.defineProperty(globalThis, 'localStorage', {
  configurable: true,
  value: {
    getItem: (key: string) => storage[key] ?? null,
    setItem: (key: string, value: string) => {
      storage[key] = value
    },
    removeItem: (key: string) => {
      delete storage[key]
    },
  },
})

const { syncTrialOutcome } = await import('./trial-outcome')
const { AnalyticsEvent, trackOncePerSession } = await import('./events')

afterAll(() => mock.restore())

type PlanState = Pick<Plan, 'status' | 'trial' | 'trialKind'>
const cardTrial: PlanState = {
  status: 'active',
  trial: true,
  trialKind: 'card',
}
const paid: PlanState = { status: 'active', trial: false }

beforeEach(() => {
  captured.length = 0
  for (const key of Object.keys(storage)) delete storage[key]
})

const names = () => captured.map((entry) => entry.event)

describe('how a card trial ended', () => {
  it('tests that a trial that turned paid is reported once, however often the plan is read', () => {
    syncTrialOutcome(cardTrial)
    syncTrialOutcome(cardTrial)
    expect(names()).toEqual([])

    syncTrialOutcome(paid)
    syncTrialOutcome(paid)
    expect(names()).toEqual(['ultrafast_trial_converted'])
  })

  it('tests that a failed first charge is reported with the plan state', () => {
    syncTrialOutcome(cardTrial)
    syncTrialOutcome({ status: 'unpaid', trial: false })
    expect(captured).toEqual([
      {
        event: 'ultrafast_trial_payment_failed',
        properties: { status: 'unpaid' },
      },
    ])
  })

  it('tests that a cancelled trial that ran out is reported as ended', () => {
    syncTrialOutcome(cardTrial)
    syncTrialOutcome({ status: 'ending', trial: true, trialKind: 'card' })
    syncTrialOutcome({ status: 'none' })
    expect(names()).toEqual(['ultrafast_trial_ended'])
  })

  it('tests that a free trial link is never watched or reported', () => {
    syncTrialOutcome({ status: 'active', trial: true, trialKind: 'grant' })
    syncTrialOutcome(paid)
    syncTrialOutcome({ status: 'none' })
    expect(names()).toEqual([])
  })

  it('tests that a plan that was never a card trial reports nothing', () => {
    syncTrialOutcome(paid)
    syncTrialOutcome({ status: 'unpaid' })
    expect(names()).toEqual([])
  })
})

describe('events seen once per page load', () => {
  it('tests that only the first one is sent', () => {
    trackOncePerSession(AnalyticsEvent.TrialTagShown, { trial_days: 7 })
    trackOncePerSession(AnalyticsEvent.TrialTagShown, { trial_days: 7 })
    expect(names()).toEqual(['ultrafast_trial_tag_shown'])
  })
})
