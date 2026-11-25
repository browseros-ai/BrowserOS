/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { afterAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { Plan } from '@browseros/claw-api'
import * as posthog from './posthog'

const captured: { event: string; properties?: Record<string, unknown> }[] = []
let ready = true
mock.module('./posthog', () => ({
  ...posthog,
  isCapturing: () => ready,
  capture: (event: string, properties?: Record<string, unknown>) => {
    if (ready) captured.push({ event, properties })
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
  ready = true
  captured.length = 0
  for (const key of Object.keys(storage)) delete storage[key]
})

const names = () => captured.map((entry) => entry.event)

/** A Web Locks stand-in that runs one holder at a time, a tick apart, like two tabs. */
function exclusiveLocks() {
  const requested: { name: string; mode?: string }[] = []
  let queue: Promise<unknown> = Promise.resolve()
  return {
    requested,
    request: (
      name: string,
      options: { mode?: string },
      run: () => unknown,
    ): Promise<unknown> => {
      requested.push({ name, mode: options.mode })
      const turn = queue.then(
        () => new Promise((resolve) => setTimeout(() => resolve(run()), 0)),
      )
      queue = turn.catch(() => undefined)
      return turn
    },
  }
}

describe('how a card trial ended', () => {
  it('tests that a trial that turned paid is reported once, however often the plan is read', async () => {
    await syncTrialOutcome(cardTrial)
    await syncTrialOutcome(cardTrial)
    expect(names()).toEqual([])

    await syncTrialOutcome(paid)
    await syncTrialOutcome(paid)
    expect(names()).toEqual(['ultrafast_trial_converted'])
  })

  it('tests that a failed first charge is reported with the plan state', async () => {
    await syncTrialOutcome(cardTrial)
    await syncTrialOutcome({ status: 'unpaid', trial: false })
    expect(captured).toEqual([
      {
        event: 'ultrafast_trial_payment_failed',
        properties: { status: 'unpaid' },
      },
    ])
  })

  it('tests that a cancelled trial that ran out is reported as ended', async () => {
    await syncTrialOutcome(cardTrial)
    await syncTrialOutcome({ status: 'ending', trial: true, trialKind: 'card' })
    await syncTrialOutcome({ status: 'none' })
    expect(names()).toEqual(['ultrafast_trial_ended'])
  })

  it('tests that a free trial link is never watched or reported', async () => {
    await syncTrialOutcome({
      status: 'active',
      trial: true,
      trialKind: 'grant',
    })
    await syncTrialOutcome(paid)
    await syncTrialOutcome({ status: 'none' })
    expect(names()).toEqual([])
  })

  it('tests that a plan read before PostHog is ready keeps the outcome for later', async () => {
    await syncTrialOutcome(cardTrial)
    ready = false
    await syncTrialOutcome(paid)
    expect(names()).toEqual([])
    expect(storage['ultrafastTrialWatch:v1']).toBe('true')

    ready = true
    await syncTrialOutcome(paid)
    await syncTrialOutcome(paid)
    expect(names()).toEqual(['ultrafast_trial_converted'])
    expect(storage['ultrafastTrialWatch:v1']).toBeUndefined()
  })

  it('tests that a plan that was never a card trial reports nothing', async () => {
    await syncTrialOutcome(paid)
    await syncTrialOutcome({ status: 'unpaid' })
    expect(names()).toEqual([])
  })
})

describe('two tabs reading the plan at once', () => {
  it('tests that the outcome is claimed by one of them under an exclusive lock', async () => {
    const locks = exclusiveLocks()
    const previous = Object.getOwnPropertyDescriptor(globalThis, 'navigator')
    Object.defineProperty(globalThis, 'navigator', {
      configurable: true,
      value: { locks },
    })
    try {
      storage['ultrafastTrialWatch:v1'] = 'true'
      await Promise.all([syncTrialOutcome(paid), syncTrialOutcome(paid)])
    } finally {
      if (previous) Object.defineProperty(globalThis, 'navigator', previous)
      else Reflect.deleteProperty(globalThis, 'navigator')
    }

    expect(names()).toEqual(['ultrafast_trial_converted'])
    expect(locks.requested).toEqual([
      { name: 'ultrafast-trial-outcome', mode: 'exclusive' },
      { name: 'ultrafast-trial-outcome', mode: 'exclusive' },
    ])
  })
})

describe('events seen once per page load', () => {
  it('tests that only the first one is sent', () => {
    trackOncePerSession(AnalyticsEvent.TrialTagShown, { trial_days: 7 })
    trackOncePerSession(AnalyticsEvent.TrialTagShown, { trial_days: 7 })
    expect(names()).toEqual(['ultrafast_trial_tag_shown'])
  })

  it('tests that one seen before PostHog is ready is sent once it is', () => {
    ready = false
    trackOncePerSession(AnalyticsEvent.TrialBarShown, { trial_days: 7 })
    expect(names()).toEqual([])

    ready = true
    trackOncePerSession(AnalyticsEvent.TrialBarShown, { trial_days: 7 })
    trackOncePerSession(AnalyticsEvent.TrialBarShown, { trial_days: 7 })
    expect(names()).toEqual(['ultrafast_trial_bar_shown'])
  })
})
