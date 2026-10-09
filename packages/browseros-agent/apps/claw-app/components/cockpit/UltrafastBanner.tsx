/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Cockpit entry to the Ultrafast waitlist. Impressions are not tracked: the
 * cockpit is the new tab page, and the waitlist's own view event is the
 * funnel's denominator.
 */

import { ArrowRight, Zap } from 'lucide-react'
import { Link } from 'react-router'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { useUltrafastJoined } from '@/screens/ultrafast/ultrafast-price'

export function UltrafastBanner() {
  const joined = useUltrafastJoined()
  if (joined) return null
  return (
    <Link
      to="/ultrafast"
      onClick={() => track(AnalyticsEvent.UltrafastBannerClicked)}
      className="group flex min-h-12 flex-wrap items-center gap-x-5 gap-y-2 rounded-lg border border-cyanotype-border bg-accent-tint/50 px-[18px] py-2.5 text-[13px] leading-5 focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-cyanotype-blue focus-visible:ring-offset-2"
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
  )
}
