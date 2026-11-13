/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { CockpitStats } from '@browseros/claw-api'
import { ArrowRight } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { Button } from '@/components/motion/button/base'
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

/** Time and AI tokens neo saved, one line under the greeting with the token split drawn beneath. */
export function SavingsCard({
  stats,
  className,
}: {
  stats: CockpitStats
  className?: string
}) {
  const { humanTimeSavedMs } = stats.allTime
  const usage = usageComparison(stats, stats.allTime)

  return (
    <section
      aria-label="Savings"
      className={cn('@container flex flex-col gap-3.5', className)}
    >
      <div className="flex items-center justify-between gap-4">
        <div className="flex min-w-0 flex-wrap items-center gap-x-[22px] gap-y-1">
          {roundTimeSaved(humanTimeSavedMs) ? (
            <Stat value={timeSavedAmount(humanTimeSavedMs)} label="saved" />
          ) : (
            <p className="text-[14px] text-ink-2">
              Your time saved starts here
            </p>
          )}
          {usage ? (
            <>
              {/* Narrower, the figures can wrap and the rule would dangle at a line end. */}
              <span
                aria-hidden
                className="@2xl:block hidden h-[18px] w-px shrink-0 bg-border-2"
              />
              <Stat
                value={formatTokens(usage.saved)}
                label={
                  <>
                    AI tokens saved ·{' '}
                    <span className="font-semibold text-accent-ink">
                      {usage.percentLess}% fewer
                    </span>
                  </>
                }
              />
            </>
          ) : null}
        </div>
        <SavingsDialog stats={stats} />
      </div>
      {usage ? (
        <div
          role="img"
          aria-label={usageLabel(usage)}
          className="-mt-0.5 flex flex-col gap-1.5"
        >
          <SplitBar usage={usage} className="h-1.5 gap-0.5" />
          <div className="flex justify-between gap-3 text-[12.5px] leading-4">
            <span className="font-semibold text-accent-ink">
              saved {formatTokens(usage.saved)}
            </span>
            <span className="text-ink-3">
              neo used {formatTokens(usage.withNeo)}
            </span>
          </div>
        </div>
      ) : null}
    </section>
  )
}

function Stat({ value, label }: { value: string; label: ReactNode }) {
  return (
    <p className="flex items-baseline gap-1.5">
      <span className="font-extrabold text-[20px] text-cyanotype-ink leading-6 tracking-[-0.015em]">
        {value}
      </span>
      <span className="text-[14px] text-ink-2 leading-[18px]">{label}</span>
    </p>
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
          className="-my-[5px] -mr-2 h-[34px] shrink-0 font-semibold text-[13.5px] text-ink-2"
        >
          Details
          <ArrowRight aria-hidden className="size-[15px]" />
        </Button>
      </CenterMorphModalTrigger>
      <CenterMorphModalContent
        ariaLabel="Your savings"
        showCloseButton
        className="max-w-[560px] border-border-2 bg-card"
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
          {usage ? <UsageMeter usage={usage} /> : null}
          <p className="text-[13.5px] text-ink-3 leading-5">
            neo gives your AI agents short page descriptions instead of
            screenshots, so they use fewer tokens, the units AI is paid in.
          </p>
        </div>
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}

function Figure({ value, label }: { value: string; label: string }) {
  return (
    <div className="flex flex-col-reverse gap-0.5 rounded-md bg-bg-canvas px-4 py-3.5">
      <dt className="text-[13.5px] text-ink-2">{label}</dt>
      <dd className="font-bold text-[24px] text-cyanotype-ink tracking-[-0.02em]">
        {value}
      </dd>
    </div>
  )
}

function UsageMeter({ usage }: { usage: UsageComparison }) {
  return (
    <div
      role="img"
      aria-label={usageLabel(usage)}
      className="flex flex-col gap-2.5"
    >
      <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
        <span className="text-[14px] text-ink-2 leading-[18px]">
          <strong className="font-extrabold text-[17px] text-cyanotype-ink tracking-[-0.01em]">
            {formatTokens(usage.saved)}
          </strong>{' '}
          AI tokens saved
        </span>
        <span className="font-semibold text-[13.5px] text-accent-ink leading-[18px]">
          {usage.percentLess}% fewer
        </span>
      </div>
      <SplitBar usage={usage} className="h-2 gap-[3px]" />
      <div className="flex justify-between gap-3 text-[13px] leading-4">
        <span className="font-semibold text-accent-ink">saved</span>
        <span className="text-ink-2">
          neo used{' '}
          <span className="font-semibold text-ink">
            {formatTokens(usage.withNeo)}
          </span>
        </span>
      </div>
    </div>
  )
}

/** The token split, darker share first: saved in blue from the left, what neo used after it. */
function SplitBar({
  usage,
  className,
}: {
  usage: UsageComparison
  className?: string
}) {
  return (
    <div className={cn('flex', className)}>
      <span
        className="rounded-[2px] bg-primary"
        style={{ flexBasis: barWidth(usage.saved, usage.withoutNeo) }}
      />
      <span className="flex-1 rounded-[2px] bg-border-2" />
    </div>
  )
}

function usageLabel(usage: UsageComparison): string {
  return `${formatTokens(usage.saved)} AI tokens saved, ${usage.percentLess}% fewer. neo used ${formatTokens(usage.withNeo)}.`
}
