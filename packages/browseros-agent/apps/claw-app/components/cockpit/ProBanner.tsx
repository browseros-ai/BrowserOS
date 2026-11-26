/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Home's way to the ultrafast page, for Free installs on a build that sells
 * Pro. The plain bar's impressions are not tracked: the cockpit is the new tab
 * page, and the Pro page's own view event is the funnel's denominator. The
 * free trial bar's are, once per page load, as the top of the trial funnel.
 */

import { ArrowRight, Sparkles } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useHref } from 'react-router'
import { ButtonLink } from '@/components/motion/button/base'
import { useCaptureReady } from '@/modules/analytics/capture.hooks'
import {
  AnalyticsEvent,
  track,
  trackOncePerSession,
} from '@/modules/analytics/events'
import { usePlan, usePlanPricing } from '@/modules/api/plan.hooks'
import { trialOfferDays } from '@/modules/plan/trial.helpers'
import { formatPrice, proPrices } from '@/screens/pro/pro.helpers'
import { PromoCard } from './PromoCard'

export const DISMISS_KEY = 'proBannerDismissed:v1'
// Its own key, so people who hid the old bar still see the free trial once.
export const TRIAL_DISMISS_KEY = 'ultrafastTrialBarDismissed:v1'

function readDismissed(key: string): boolean {
  try {
    return localStorage.getItem(key) === 'true'
  } catch {
    return false
  }
}

/** Whether both the ultrafast bar and its free trial bar were hidden for good. */
export function readProBannerDismissed(): boolean {
  return readDismissed(DISMISS_KEY) && readDismissed(TRIAL_DISMISS_KEY)
}

export function ProBanner({ onClosed }: { onClosed?: () => void }) {
  const plan = usePlan()
  const pricing = usePlanPricing()
  const proHref = useHref('/pro')
  const capturing = useCaptureReady()
  const trialDays = trialOfferDays(plan.data, pricing.data)
  const key = trialDays > 0 ? TRIAL_DISMISS_KEY : DISMISS_KEY
  const [hidden, setHidden] = useState<readonly string[]>(() =>
    [DISMISS_KEY, TRIAL_DISMISS_KEY].filter(readDismissed),
  )
  const dismissed = hidden.includes(key)
  const month = pricing.data ? proPrices(pricing.data)?.month : undefined
  const trialShown = trialDays > 0 && !dismissed && month !== undefined

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === key && readDismissed(key)) {
        setHidden((previous) => [...previous, key])
        onClosed?.()
      }
    }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  }, [key, onClosed])

  // Reports what was on screen, which only a rendered bar knows; re-runs when
  // PostHog becomes ready, so an impression before then still goes out.
  useEffect(() => {
    if (trialShown && capturing) {
      trackOncePerSession(AnalyticsEvent.TrialBarShown, {
        trial_days: trialDays,
      })
    }
  }, [trialShown, trialDays, capturing])

  const dismiss = () => {
    if (trialDays > 0) track(AnalyticsEvent.TrialBarDismissed)
    setHidden((previous) => [...previous, key])
    try {
      localStorage.setItem(key, 'true')
    } catch {
      // Still hide this tab's banner when browser storage is unavailable.
    }
    onClosed?.()
  }

  // The line quotes the monthly price, so the bar waits for it and stays away
  // when the prices can't be read.
  if (dismissed || pricing.isPending || pricing.isError || !month) return null
  const price = formatPrice(month)
  return (
    <PromoCard
      aria-label="ultrafast"
      icon={<Sparkles />}
      title={
        <>
          <em className="text-accent-ink">ultrafast</em>
        </>
      }
      description={
        trialDays > 0
          ? `Try it free for ${trialDays} days, then ${price} a month.`
          : `Faster skills and more, for ${price} a month.`
      }
      dismissLabel="Hide this"
      onDismiss={dismiss}
      action={
        <ButtonLink
          href={proHref}
          size="sm"
          variant="ghost"
          onClick={() => {
            track(AnalyticsEvent.UltrafastBannerClicked)
            if (trialDays > 0) {
              track(AnalyticsEvent.TrialStartClicked, { source: 'home_bar' })
            }
          }}
          className="h-8 px-2.5 font-semibold text-[13.5px] text-accent-ink hover:bg-card/70 hover:text-accent-ink"
        >
          {trialDays > 0 ? 'Start free trial' : 'See ultrafast'}
          <ArrowRight aria-hidden className="size-[15px]" />
        </ButtonLink>
      }
    />
  )
}
