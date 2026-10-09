/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Cockpit entry to the Ultrafast waitlist. Impressions are not tracked: the
 * cockpit is the new tab page, and the waitlist's own view event is the
 * funnel's denominator.
 */

import { ArrowRight, X, Zap } from 'lucide-react'
import { useEffect, useState } from 'react'
import { Link } from 'react-router'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { useUltrafastJoined } from '@/screens/ultrafast/ultrafast-price'

const DISMISS_KEY = 'ultrafastBannerDismissed:v1'

function readDismissed(): boolean {
  try {
    return localStorage.getItem(DISMISS_KEY) === 'true'
  } catch {
    return false
  }
}

export function UltrafastBanner() {
  const joined = useUltrafastJoined()
  const [dismissed, setDismissed] = useState(readDismissed)

  useEffect(() => {
    const onStorage = (event: StorageEvent) => {
      if (event.key === DISMISS_KEY && readDismissed()) setDismissed(true)
    }
    window.addEventListener('storage', onStorage)
    return () => window.removeEventListener('storage', onStorage)
  }, [])

  const dismiss = () => {
    setDismissed(true)
    try {
      localStorage.setItem(DISMISS_KEY, 'true')
    } catch {
      // Still hide this tab's banner when browser storage is unavailable.
    }
  }

  if (joined || dismissed) return null
  return (
    <div className="relative rounded-lg border border-cyanotype-border bg-accent-tint/50">
      <Link
        to="/ultrafast"
        onClick={() => track(AnalyticsEvent.UltrafastBannerClicked)}
        className="group flex min-h-12 flex-wrap items-center gap-x-5 gap-y-2 rounded-lg py-2.5 pr-14 pl-[18px] text-[13px] leading-5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyanotype-blue focus-visible:ring-offset-2"
      >
        <span className="flex shrink-0 items-center gap-2">
          <Zap aria-hidden="true" className="size-4 text-cyanotype-blue" />
          <span className="font-[650] text-[16px] text-cyanotype-ink leading-[22px] tracking-[-0.035em]">
            <span className="text-cyanotype-blue italic">Ultrafast</span> mode
          </span>
        </span>
        <span className="order-last w-full text-cyanotype-soft md:order-none md:w-auto md:min-w-0 md:flex-1 md:border-cyanotype-border md:border-l md:pl-5">
          Faster browser tasks. 10× fewer tokens from your main agent.
        </span>
        <span className="ml-auto flex shrink-0 items-center gap-2 font-medium text-cyanotype-blue underline-offset-4 group-hover:underline">
          Request early access
          <ArrowRight aria-hidden="true" className="size-4" />
        </span>
      </Link>
      <button
        type="button"
        onClick={dismiss}
        aria-label="Dismiss this banner"
        title="Dismiss this banner"
        className="absolute top-2 right-2 flex size-8 cursor-pointer items-center justify-center rounded-md border border-cyanotype-border bg-background text-cyanotype-ink transition-colors hover:bg-accent-tint focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyanotype-blue focus-visible:ring-offset-2"
      >
        <X aria-hidden="true" className="size-4" />
      </button>
    </div>
  )
}
