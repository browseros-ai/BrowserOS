/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { CockpitStats } from '@browseros/claw-api'
import { ArrowRight, Clock } from 'lucide-react'
import { useState } from 'react'
import { AnimatedNumber } from '@/components/motion/animated-number'
import { Button } from '@/components/motion/button/base'
import { Card } from '@/components/motion/card'
import {
  CenterMorphModal,
  CenterMorphModalContent,
  CenterMorphModalTrigger,
} from '@/components/motion/center-morph-modal'
import { Tabs, TabsList, TabsTrigger } from '@/components/motion/tabs'
import { cn } from '@/lib/utils'
import { roundTimeSaved } from '@/screens/cockpit/cockpit.helpers'
import {
  barWidth,
  formatTokens,
  SAVINGS_WINDOWS,
  type SavingsWindowKey,
  timeSavedAmount,
  type UsageComparison,
  usageComparison,
} from './savings.helpers'

/**
 * Time saved in words, with AI usage with and without neo drawn underneath.
 * Token counts stay in Details, so the picture carries the point on its own.
 */
export function SavingsCard({ stats }: { stats: CockpitStats }) {
  const { sessionCount, humanTimeSavedMs } = stats.allTime
  const saved = roundTimeSaved(humanTimeSavedMs)
  const tasks = sessionCount === 1 ? 'task' : 'tasks'
  const usage = usageComparison(stats, stats.allTime)

  return (
    <Card
      size="compact"
      role="region"
      aria-label="Savings"
      className="gap-4 border-border-2 bg-card px-[18px] pt-4 pb-[18px] shadow-card"
    >
      <div className="flex flex-wrap items-center gap-x-3.5 gap-y-1.5">
        <span
          aria-hidden
          className="grid size-[42px] shrink-0 place-items-center rounded-full bg-green-tint text-green-ink"
        >
          <Clock className="size-[18px]" strokeWidth={1.75} />
        </span>
        <div className="flex min-w-0 flex-1 basis-[calc(100%-56px)] flex-col gap-px md:basis-auto">
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
        <SavingsDialog stats={stats} />
      </div>
      {usage ? <UsageBars usage={usage} className="md:pl-14" /> : null}
    </Card>
  )
}

// ---------------------------------------------------------------------------

function SavingsDialog({ stats }: { stats: CockpitStats }) {
  const [windowKey, setWindowKey] = useState<SavingsWindowKey>('allTime')
  const window = stats[windowKey]
  const usage = usageComparison(stats, window)

  return (
    <CenterMorphModal>
      <CenterMorphModalTrigger>
        <Button
          variant="ghost"
          size="sm"
          className="ml-14 h-[34px] shrink-0 font-semibold text-[13.5px] text-ink-2 md:ml-auto"
        >
          Details
          <ArrowRight aria-hidden className="size-[15px]" />
        </Button>
      </CenterMorphModalTrigger>
      <CenterMorphModalContent
        ariaLabel="Your savings"
        showCloseButton
        className="max-w-[560px] rounded-[22px] border-border-2 bg-card"
      >
        <div className="flex flex-col gap-[18px] px-6 pt-6 pb-6">
          <h2 className="pr-10 font-bold text-[20px] text-cyanotype-ink tracking-[-0.015em]">
            Your savings
          </h2>
          <Tabs
            variant="segment"
            value={windowKey}
            onValueChange={(value) => setWindowKey(value as SavingsWindowKey)}
          >
            <TabsList aria-label="Period">
              {SAVINGS_WINDOWS.map(({ key, label }) => (
                <TabsTrigger key={key} value={key}>
                  {label}
                </TabsTrigger>
              ))}
            </TabsList>
          </Tabs>
          <dl className="grid grid-cols-1 gap-3 sm:grid-cols-3">
            <Figure
              value={timeSavedAmount(window.humanTimeSavedMs)}
              label="of your time saved"
            />
            <Figure
              value={window.sessionCount.toLocaleString()}
              label="tasks finished"
            />
            <Figure
              value={window.toolCallCount.toLocaleString()}
              label="steps taken"
            />
          </dl>
          {usage ? <UsageBars usage={usage} withNumbers /> : null}
          <p className="text-[13.5px] text-ink-3">
            AI usage is counted in tokens, the small pieces of text an AI reads
            and writes. neo hands your AI agents short descriptions of each page
            instead of screenshots, so they read far less. “Without neo” is our
            estimate for the same tasks done from screenshots.
          </p>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}

function Figure({ value, label }: { value: string; label: string }) {
  return (
    <div className="flex flex-col-reverse gap-0.5 rounded-[14px] bg-bg-canvas px-4 py-3.5">
      <dt className="text-[13.5px] text-ink-2">{label}</dt>
      <dd className="font-bold text-[24px] text-cyanotype-ink tracking-[-0.02em]">
        {value}
      </dd>
    </div>
  )
}

function UsageBars({
  usage,
  withNumbers = false,
  className,
}: {
  usage: UsageComparison
  withNumbers?: boolean
  className?: string
}) {
  const rows = [
    { label: 'With neo', value: usage.withNeo, bar: 'bg-green' },
    {
      label: 'Without neo',
      value: usage.withoutNeo,
      bar: 'bg-ink-3/30',
    },
  ]
  return (
    <div
      role="img"
      aria-label={`AI usage: ${usage.percentLess}% less with neo than without`}
      className={cn('grid gap-2', withNumbers && 'gap-3', className)}
    >
      <div className="flex items-baseline justify-between font-semibold text-[13px] text-ink-3">
        <span>AI usage</span>
        <span className="font-bold text-green-ink">
          {usage.percentLess}% less
        </span>
      </div>
      {rows.map((row) => (
        <div
          key={row.label}
          className={cn(
            'grid items-center gap-3 text-[13.5px] text-ink-2',
            withNumbers
              ? 'grid-cols-[100px_minmax(0,1fr)_52px]'
              : 'grid-cols-[84px_minmax(0,1fr)] md:grid-cols-[92px_minmax(0,1fr)]',
          )}
        >
          <span>{row.label}</span>
          <span
            className={cn(
              'overflow-hidden rounded-full bg-border-2/60',
              withNumbers ? 'h-3.5' : 'h-2.5',
            )}
          >
            <span
              className={cn('block h-full rounded-full', row.bar)}
              style={{ width: barWidth(row.value, usage.withoutNeo) }}
            />
          </span>
          {withNumbers ? (
            <span className="text-right font-mono text-[13px]">
              {formatTokens(row.value)}
            </span>
          ) : null}
        </div>
      ))}
    </div>
  )
}
