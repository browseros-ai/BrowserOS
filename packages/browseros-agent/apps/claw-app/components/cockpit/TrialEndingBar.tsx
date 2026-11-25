/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Home's reminder in the last days of a card trial: the day and the amount of
 * the first charge, the same day our reminder email goes out.
 */

import type { Plan } from '@browseros/claw-api'
import { ArrowRight, CalendarClock } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useHref } from 'react-router'
import { ButtonLink } from '@/components/motion/button/base'
import {
  AnalyticsEvent,
  track,
  trackOncePerSession,
} from '@/modules/analytics/events'
import { usePlanPricing } from '@/modules/api/plan.hooks'
import {
  daysLabel,
  isCardTrial,
  TRIAL_ENDING_DAYS,
  trialDaysLeft,
} from '@/modules/plan/trial.helpers'
import { formatDate, formatPrice, proPrices } from '@/screens/pro/pro.helpers'
import { PromoCard } from './PromoCard'

const DISMISS_KEY = 'ultrafastTrialEndingBarDismissed:v1'

function readDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISS_KEY) === 'true'
  } catch {
    return false
  }
}

/**
 * Whether Home owes this plan the reminder: a card trial that will turn into
 * a paid plan within the last few days. A cancelled trial won't be charged, so
 * it gets none.
 */
export function isTrialEndingDue(plan: Plan | undefined, now: number): boolean {
  if (!isCardTrial(plan) || plan?.status !== 'active') return false
  const left = trialDaysLeft(plan, now)
  return left !== null && left <= TRIAL_ENDING_DAYS && !readDismissed()
}

export function TrialEndingBar({
  plan,
  onClosed,
}: {
  plan: Plan
  onClosed?: () => void
}) {
  const pricing = usePlanPricing()
  const plansHref = useHref('/pro')
  const [dismissed, setDismissed] = useState(readDismissed)
  const left = trialDaysLeft(plan, Date.now())
  const shown = !dismissed && left !== null && plan.trialEndsAt !== undefined

  // Reports what was on screen, which only a rendered bar knows.
  useEffect(() => {
    if (shown && left !== null) {
      trackOncePerSession(AnalyticsEvent.TrialEndingBarShown, {
        days_left: left,
      })
    }
  }, [shown, left])

  if (!shown || left === null || plan.trialEndsAt === undefined) return null

  const month = pricing.data ? proPrices(pricing.data)?.month : undefined
  const date = formatDate(plan.trialEndsAt)
  const dismiss = () => {
    track(AnalyticsEvent.TrialEndingBarDismissed, { days_left: left })
    setDismissed(true)
    try {
      localStorage.setItem(DISMISS_KEY, 'true')
    } catch {
      // Still hide this tab's bar when browser storage is unavailable.
    }
    onClosed?.()
  }

  return (
    <PromoCard
      aria-label="Free trial"
      icon={<CalendarClock />}
      title="Free trial"
      description={`${daysLabel(Math.max(1, left))} left. ${
        month
          ? `We’ll charge ${formatPrice(month)} on ${date}.`
          : `We’ll charge you on ${date}.`
      }`}
      dismissLabel="Hide this"
      onDismiss={dismiss}
      action={
        <ButtonLink
          href={plansHref}
          size="sm"
          variant="ghost"
          onClick={() =>
            track(AnalyticsEvent.TrialEndingBarClicked, { days_left: left })
          }
          className="h-8 px-2.5 font-semibold text-[13.5px] text-accent-ink hover:bg-card/70 hover:text-accent-ink"
        >
          See your plan
          <ArrowRight aria-hidden className="size-[15px]" />
        </ButtonLink>
      }
    />
  )
}
