/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Home's way to the neo Pro page, for Free installs on a build that sells
 * Pro. Impressions are not tracked: the cockpit is the new tab page, and the
 * Pro page's own view event is the funnel's denominator.
 */

import { ArrowRight, Sparkles } from 'lucide-react'
import { useEffect, useState } from 'react'
import { useHref } from 'react-router'
import { ButtonLink } from '@/components/motion/button/base'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { usePlanPricing } from '@/modules/api/plan.hooks'
import { formatPrice, proPrices } from '@/screens/pro/pro.helpers'
import { PromoCard } from './PromoCard'

const DISMISS_KEY = 'proBannerDismissed:v1'

export function readProBannerDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISS_KEY) === 'true'
  } catch {
    return false
  }
}

export function ProBanner({ onClosed }: { onClosed?: () => void }) {
  const pricing = usePlanPricing()
  const [dismissed, setDismissed] = useState(readProBannerDismissed)
  const proHref = useHref('/pro')

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === DISMISS_KEY && readProBannerDismissed()) {
        setDismissed(true)
        onClosed?.()
      }
    }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  }, [onClosed])

  const dismiss = () => {
    setDismissed(true)
    try {
      localStorage.setItem(DISMISS_KEY, 'true')
    } catch {
      // Still hide this tab's banner when browser storage is unavailable.
    }
    onClosed?.()
  }

  // The line quotes the monthly price, so the bar waits for it and stays away
  // when the prices can't be read.
  const month = pricing.data ? proPrices(pricing.data)?.month : undefined
  if (dismissed || pricing.isPending || pricing.isError || !month) return null
  return (
    <PromoCard
      aria-label="neo Pro"
      icon={<Sparkles />}
      title={
        <>
          neo <em className="text-accent-ink">Pro</em>
        </>
      }
      description={`Faster tasks, a year of history and more, for ${formatPrice(month)} a month.`}
      dismissLabel="Hide this"
      onDismiss={dismiss}
      action={
        <ButtonLink
          href={proHref}
          size="sm"
          variant="ghost"
          onClick={() => track(AnalyticsEvent.UltrafastBannerClicked)}
          className="h-8 px-2.5 font-semibold text-[13.5px] text-accent-ink hover:bg-card/70 hover:text-accent-ink"
        >
          See Pro
          <ArrowRight aria-hidden className="size-[15px]" />
        </ButtonLink>
      }
    />
  )
}
