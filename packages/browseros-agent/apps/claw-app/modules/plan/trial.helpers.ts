/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The 7-day free trial, as the app shows it. The gateway decides who gets a
 * trial; these helpers only read what it already said. A trial from a free
 * trial link (`trialKind: grant`) is shown as a paid plan, so every trial
 * screen asks `isCardTrial` first. No React, no side effects.
 */

import type { Plan, PlanPricing } from '@browseros/claw-api'

const DAY_MS = 86_400_000

/** A trial the person started with a card, the only kind the app shows as a trial. */
export function isCardTrial(
  plan: Pick<Plan, 'trial' | 'trialKind'> | undefined,
): boolean {
  return plan?.trial === true && plan.trialKind === 'card'
}

/** The trial days the monthly price is offered with, or 0 when it has none. */
export function monthlyTrialDays(pricing: PlanPricing | undefined): number {
  const sold =
    pricing?.plans.find((plan) => plan.plan === 'ultrafast') ??
    pricing?.plans[0]
  const month = sold?.prices.find(
    (price) => price.interval === 'month' && price.intervalCount === 1,
  )
  return Math.max(0, month?.trialDays ?? 0)
}

/**
 * The trial days to offer this computer, or 0. Only Free installs that were
 * never on a key see it: someone whose key lapsed already had ultrafast, so
 * they get Renew instead. The gateway still checks at checkout.
 */
export function trialOfferDays(
  plan: Pick<Plan, 'available' | 'status' | 'onKey'> | undefined,
  pricing: PlanPricing | undefined,
): number {
  if (!plan?.available || plan.status !== 'none' || plan.onKey) return 0
  return monthlyTrialDays(pricing)
}

/** Whole days left in a card trial, rounded up so the last day reads "1 day left". */
export function trialDaysLeft(
  plan: Pick<Plan, 'trial' | 'trialKind' | 'trialEndsAt'> | undefined,
  now: number,
): number | null {
  if (!isCardTrial(plan) || plan?.trialEndsAt === undefined) return null
  return Math.max(0, Math.ceil((plan.trialEndsAt - now) / DAY_MS))
}

/** When a trial started now would end, for "Free until Dec 2, 2026". */
export function trialEndFrom(now: number, days: number): number {
  return now + days * DAY_MS
}

export function daysLabel(days: number): string {
  return days === 1 ? '1 day' : `${days} days`
}

/** From 3 days left Home carries the charge date, the day our reminder email goes out. */
export const TRIAL_ENDING_DAYS = 3
