/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { CockpitStats } from '@browseros/claw-api'
import { Clock } from 'lucide-react'
import { AnimatedNumber } from '@/components/motion/animated-number'
import { Card } from '@/components/motion/card'
import { roundTimeSaved } from '@/screens/cockpit/cockpit.helpers'

/** Time neo has saved so far, in hours people can picture rather than tokens. */
export function TimeSavedCard({ stats }: { stats: CockpitStats }) {
  const { sessionCount, humanTimeSavedMs } = stats.allTime
  const saved = roundTimeSaved(humanTimeSavedMs)
  const tasks = sessionCount === 1 ? 'task' : 'tasks'

  return (
    <Card
      size="compact"
      className="h-full flex-row items-center gap-3.5 border-border-2 bg-card px-4 py-3.5 shadow-card"
    >
      <span
        aria-hidden
        className="grid size-[42px] shrink-0 place-items-center rounded-full bg-green-tint text-green-ink"
      >
        <Clock className="size-[18px]" strokeWidth={1.75} />
      </span>
      <div className="flex min-w-0 flex-col gap-px">
        <p className="font-bold text-[15.5px] text-cyanotype-ink">
          {saved ? (
            <>
              About <AnimatedNumber value={saved.amount} />{' '}
              {saved.amount === 1 ? saved.unit : `${saved.unit}s`} saved
            </>
          ) : (
            'Your time saved starts here'
          )}
        </p>
        <p className="text-[14px] text-ink-2">
          neo has finished {sessionCount.toLocaleString()} {tasks} for you so
          far.
        </p>
      </div>
    </Card>
  )
}
