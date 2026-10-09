/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Cockpit entry to the Ultrafast waitlist. Impressions are not tracked: the
 * cockpit is the new tab page, and the waitlist's own view event is the
 * funnel's denominator.
 */

import { ArrowUpRight, Zap } from 'lucide-react'
import { Link } from 'react-router'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { readJoined } from '@/screens/ultrafast/ultrafast-price'

export function UltrafastBanner() {
  if (readJoined()) return null
  return (
    <Link
      to="/ultrafast"
      onClick={() => track(AnalyticsEvent.UltrafastBannerClicked)}
      className="group flex items-center justify-center gap-3 rounded-[9px] border border-cyanotype-border bg-accent-tint/50 px-4 py-2.5 text-[13px] leading-5"
    >
      <span className="flex items-center gap-1.5 font-semibold text-cyanotype-blue uppercase tracking-[0.06em]">
        <Zap className="size-3.5" />
        Ultrafast mode
      </span>
      <span className="flex items-center gap-0.5 text-cyanotype-ink underline-offset-4 group-hover:underline">
        Join the waitlist
        <ArrowUpRight className="size-3.5" />
      </span>
    </Link>
  )
}
