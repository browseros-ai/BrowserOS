/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ArrowRight, Gauge } from 'lucide-react'
import { type ReactNode, useEffect, useState } from 'react'
import { UpgradeDialog } from '@/components/skills/UpgradeDialog'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'

/** Where the speed nudge was shown, as analytics names it. */
export type NudgeSurface = 'skills_list' | 'skill_page'

/**
 * One quiet line for a free plan: its skills run at normal speed, and ultrafast runs them in
 * seconds. "See ultrafast" opens the upgrade dialog; this is the only ask on its screen.
 */
export function UltrafastNudge({
  surface,
  children,
  className,
}: {
  surface: NudgeSurface
  children: ReactNode
  className?: string
}) {
  const [open, setOpen] = useState(false)
  // Counting an impression is a side effect of the line appearing; no handler runs then.
  useEffect(() => countShown(surface), [surface])
  return (
    <>
      <p
        className={cn(
          'flex flex-wrap items-center gap-x-2.5 gap-y-1.5 text-[14.5px] text-ink-2',
          className,
        )}
      >
        <Gauge aria-hidden className="size-4 shrink-0 text-primary" />
        <span>{children}</span>
        <button
          type="button"
          onClick={() => {
            track(AnalyticsEvent.UltrafastNudgeClicked, { surface })
            setOpen(true)
          }}
          className="inline-flex items-center gap-1 rounded-sm font-semibold text-primary outline-none hover:underline focus-visible:ring-2 focus-visible:ring-ring"
        >
          See ultrafast
          <ArrowRight aria-hidden className="size-[15px]" />
        </button>
      </p>
      <UpgradeDialog open={open} onOpenChange={setOpen} />
    </>
  )
}

/** Surfaces whose nudge has been counted as shown in this app load: one impression each. */
const countedSurfaces = new Set<NudgeSurface>()

function countShown(surface: NudgeSurface) {
  if (countedSurfaces.has(surface)) return
  countedSurfaces.add(surface)
  track(AnalyticsEvent.UltrafastNudgeShown, { surface })
}
