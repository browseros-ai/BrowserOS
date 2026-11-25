/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * How a card trial ended, as this computer saw it: paid, payment failed, or
 * ended without paying. While a card trial is on, a mark is kept in
 * localStorage; the first plan read after the trial reports the outcome once
 * and clears the mark, so a reload or a second tab never reports it again.
 * Stripe's webhooks on the gateway stay the source of truth for revenue.
 */

import type { Plan } from '@browseros/claw-api'
import { isCardTrial } from '@/modules/plan/trial.helpers'
import { AnalyticsEvent, type AnalyticsEventName, track } from './events'
import { isCapturing } from './posthog'

export const MARK_KEY = 'ultrafastTrialWatch:v1'
// Every new tab page reads the same marker, so reading and clearing it is one
// exclusive step across tabs; without it two tabs could both report the outcome.
const LOCK_NAME = 'ultrafast-trial-outcome'

type TrialOutcome = {
  event: AnalyticsEventName
  properties?: Record<string, boolean | number | string>
}

/**
 * What to report for `plan`, given whether a card trial was on when this
 * computer last looked. `mark` is whether to keep watching.
 */
function trialOutcome(
  watching: boolean,
  plan: Pick<Plan, 'status' | 'trial' | 'trialKind'>,
): { outcome: TrialOutcome | null; mark: boolean } {
  if (isCardTrial(plan)) return { outcome: null, mark: true }
  if (!watching) return { outcome: null, mark: false }
  switch (plan.status) {
    case 'active':
      return {
        outcome: { event: AnalyticsEvent.TrialConverted },
        mark: false,
      }
    case 'paymentFailed':
    case 'unpaid':
      return {
        outcome: {
          event: AnalyticsEvent.TrialPaymentFailed,
          properties: { status: plan.status },
        },
        mark: false,
      }
    case 'ending':
      // A paid plan already set to end: the first payment went through.
      return {
        outcome: { event: AnalyticsEvent.TrialConverted },
        mark: false,
      }
    default:
      return { outcome: { event: AnalyticsEvent.TrialEnded }, mark: false }
  }
}

function readMark(): boolean {
  try {
    return localStorage.getItem(MARK_KEY) === 'true'
  } catch {
    return false
  }
}

function writeMark(on: boolean): void {
  try {
    if (on) localStorage.setItem(MARK_KEY, 'true')
    else localStorage.removeItem(MARK_KEY)
  } catch {
    // Without storage the outcome can't be told apart from a reload, so it isn't reported.
  }
}

/**
 * Reports a card trial's outcome once, on the first plan read after it ends.
 * The marker is cleared only once the outcome can be sent, so a read before
 * PostHog is ready leaves it for the next one.
 */
export async function syncTrialOutcome(
  plan: Pick<Plan, 'status' | 'trial' | 'trialKind'>,
): Promise<void> {
  const settle = () => {
    const watching = readMark()
    const { outcome, mark } = trialOutcome(watching, plan)
    if (outcome && !isCapturing()) return
    if (outcome) track(outcome.event, outcome.properties)
    if (mark !== watching) writeMark(mark)
  }
  const locks = globalThis.navigator?.locks
  if (!locks) {
    settle()
    return
  }
  try {
    await locks.request(LOCK_NAME, { mode: 'exclusive' }, settle)
  } catch {
    // A browser that refuses the lock still gets this tab's answer.
    settle()
  }
}
