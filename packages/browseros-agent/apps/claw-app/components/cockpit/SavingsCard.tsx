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

/** Time saved in words, with the AI tokens neo saved drawn and counted underneath. */
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
          {usage ? <UsageBars usage={usage} large /> : null}
          <p className="text-[13.5px] text-ink-3">
            AI tokens are the small pieces of text an AI reads and writes, and
            what AI use is measured and paid in. neo hands your AI agents short
            descriptions of each page instead of screenshots, so they use far
            fewer. “Without neo” is our estimate for the same tasks done from
            screenshots.
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

// "Without neo" is amber, the cost, rather than a gray that reads as switched off.
const WITHOUT_NEO_BAR =
  'bg-[color-mix(in_srgb,var(--color-amber)_62%,var(--color-card))]'

function UsageBars({
  usage,
  large = false,
  className,
}: {
  usage: UsageComparison
  large?: boolean
  className?: string
}) {
  const lane = cn('flex min-w-0 gap-[3px]', large ? 'h-6' : 'h-5')
  const row = cn(
    'grid items-center gap-3 text-[13.5px] text-ink-2',
    large
      ? 'grid-cols-[100px_minmax(0,1fr)_52px]'
      : 'grid-cols-[84px_minmax(0,1fr)_44px] md:grid-cols-[92px_minmax(0,1fr)_48px]',
  )
  return (
    <div
      role="img"
      aria-label={`${formatTokens(usage.saved)} AI tokens saved, ${usage.percentLess}% less with neo than without`}
      className={cn('grid gap-2.5', className)}
    >
      <div className="flex flex-wrap items-center justify-between gap-x-3 gap-y-1">
        <span className="text-[14px] text-ink-2">
          <strong className="font-bold text-[16px] text-cyanotype-ink tracking-[-0.01em]">
            {formatTokens(usage.saved)}
          </strong>{' '}
          AI tokens saved
        </span>
        <span className="rounded-full bg-green-tint px-2.5 py-0.5 font-semibold text-[12.5px] text-green-ink">
          {usage.percentLess}% less
        </span>
      </div>
      <div className={row}>
        <span>With neo</span>
        <span className={lane}>
          <span
            className="shrink rounded-full bg-green"
            style={{ flexBasis: barWidth(usage.withNeo, usage.withoutNeo) }}
          />
          <span
            className="grid shrink place-items-center overflow-hidden rounded-full border-[1.5px] border-green/70 border-dashed bg-green/10 font-semibold text-[11.5px] text-green-ink"
            style={{ flexBasis: barWidth(usage.saved, usage.withoutNeo) }}
          >
            <span className="truncate px-1.5">saved</span>
          </span>
        </span>
        <span className="text-right font-semibold text-[13px] text-ink">
          {formatTokens(usage.withNeo)}
        </span>
      </div>
      <div className={row}>
        <span>Without neo</span>
        <span className={lane}>
          <span className={cn('w-full rounded-full', WITHOUT_NEO_BAR)} />
        </span>
        <span className="text-right font-semibold text-[13px] text-ink">
          {formatTokens(usage.withoutNeo)}
        </span>
      </div>
    </div>
  )
}
