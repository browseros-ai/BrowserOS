/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Home for an ultrafast customer: the tokens saved since this computer got ultrafast, with four
 * quieter figures beside it, on the ultrafast ink. Free installs keep SavingsCard.
 */

import type { PlanSavings, PlanSavingsMeter } from '@browseros/claw-api'
import type { UseQueryResult } from '@tanstack/react-query'
import { ArrowRight, Copy } from 'lucide-react'
import { type ReactNode, useState } from 'react'
import { UltrafastWordmark } from '@/components/audit/UltrafastMarks'
import { cn } from '@/lib/utils'
import { AnalyticsEvent, track } from '@/modules/analytics/events'
import { ultrafastTime } from '@/screens/audit/audit.helpers'
import {
  milestoneNote,
  tokensSavedSinceUltrafast,
} from './energy-meter.helpers'
import { BigNumber, INK, InkFact, PANEL_SHADOW } from './InkPanel'
import { timeSavedAmount } from './savings.helpers'
import { UltrafastDetailsDialog } from './UltrafastDetailsDialog'
import { monthDay } from './ultrafast-details.helpers'

export const TRY_TASK =
  'On google.com/travel/flights, search one-way flights from Chennai to Singapore on 20 December and sort by price.'

const WORDMARK = cn('text-[16px]', INK.ink)

export function UltrafastSummary({
  savings,
  className,
}: {
  savings: UseQueryResult<PlanSavings>
  className?: string
}) {
  if (savings.isPending) return <Loading className={className} />
  const meter = savings.data?.meter
  const tokens = savings.data ? tokensSavedSinceUltrafast(savings.data) : null
  if (savings.isError || !meter || tokens === null) {
    return (
      <Panel className={className}>
        <div className="flex flex-col items-start gap-3">
          <UltrafastWordmark className={WORDMARK} />
          <p role="alert" className={cn('m-0 text-[14.5px]', INK.ink2)}>
            Your ultrafast numbers didn't load.
          </p>
          <button
            type="button"
            onClick={() => savings.refetch()}
            className={cn(
              'h-[34px] rounded-lg border px-3.5 font-semibold text-[13.5px] hover:bg-[rgb(238_243_255/0.07)] active:translate-y-px',
              INK.line,
              INK.ink,
            )}
          >
            Try again
          </button>
        </div>
      </Panel>
    )
  }
  if (savings.data.tasks === 0) return <FirstDay className={className} />
  return (
    <Summary
      savings={savings.data}
      meter={meter}
      tokens={tokens}
      className={className}
    />
  )
}

/** The ink panel. A container query sizes the children of `@container`, so the wrapper declares it. */
function Panel({
  children,
  className,
  label = 'ultrafast',
  busy = false,
}: {
  children: ReactNode
  className?: string
  label?: string
  busy?: boolean
}) {
  return (
    <div className={cn('@container', className)}>
      <section
        aria-label={label}
        aria-busy={busy || undefined}
        className={cn(
          'relative rounded-2xl @2xl:px-7 px-[18px] @2xl:pt-[22px] pt-[18px] @2xl:pb-[26px] pb-5',
          INK.panel,
          PANEL_SHADOW,
        )}
      >
        {children}
      </section>
    </div>
  )
}

function Summary({
  savings,
  meter,
  tokens,
  className,
}: {
  savings: PlanSavings
  meter: PlanSavingsMeter
  tokens: number
  className?: string
}) {
  const note = milestoneNote(tokens, savings.tasks)
  return (
    <Panel className={className}>
      <div className="mb-3.5 flex items-baseline justify-between gap-3">
        <div className="flex items-baseline gap-2.5">
          <UltrafastWordmark className={WORDMARK} />
          <span className={cn('text-[13.5px]', INK.ink3)}>
            since {monthDay(meter.since)}
          </span>
        </div>
        <UltrafastDetailsDialog
          savings={savings}
          trigger={
            <button
              type="button"
              onClick={() => track(AnalyticsEvent.UltrafastDetailsOpened)}
              className={cn(
                '-my-[5px] -mr-2 inline-flex h-[34px] shrink-0 items-center gap-1.5 rounded-lg px-2 font-semibold text-[13.5px] hover:bg-[rgb(238_243_255/0.08)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#9cc0ff] active:translate-y-px',
                INK.ink,
              )}
            >
              Details
              <ArrowRight aria-hidden className="size-3.5" />
            </button>
          }
        />
      </div>
      <div className="grid @2xl:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)] grid-cols-1 items-end gap-x-12 gap-y-[22px]">
        <div className="flex flex-col gap-2.5">
          <div className="flex flex-wrap items-baseline gap-x-3.5 gap-y-1">
            <BigNumber
              tokens={tokens}
              className="@2xl:text-[68px] text-[54px]"
            />
            <span className={cn('text-[15px]', INK.ink2)}>tokens saved</span>
          </div>
          {note ? (
            <span className={cn('font-semibold text-[13.5px]', INK.accent)}>
              {note}
            </span>
          ) : null}
        </div>
        <dl className="m-0 grid grid-cols-2 gap-x-8 gap-y-5">
          <InkFact
            label="time saved"
            value={timeSavedAmount(meter.periods.since.timeSavedMs)}
          />
          <InkFact
            label={savings.tasks === 1 ? 'ultrafast run' : 'ultrafast runs'}
            value={savings.tasks.toLocaleString('en-US')}
          />
          <InkFact
            label="steps done by neo"
            value={
              meter.neoStepShare == null ? 'None yet' : `${meter.neoStepShare}%`
            }
          />
          <InkFact
            label="fastest run"
            value={
              meter.fastestRunMs == null
                ? 'None yet'
                : ultrafastTime(meter.fastestRunMs)
            }
          />
        </dl>
      </div>
    </Panel>
  )
}

/** Before the first run there is nothing to count yet, so the card hands over a task to try. */
function FirstDay({ className }: { className?: string }) {
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(TRY_TASK)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  return (
    <Panel className={className}>
      <div className="mb-3.5">
        <UltrafastWordmark className={WORDMARK} />
      </div>
      <div className="grid @2xl:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)] grid-cols-1 items-center gap-x-12 gap-y-[22px]">
        <div className="flex flex-col gap-2.5">
          <h3
            className={cn(
              'm-0 font-bold @2xl:text-[28px] text-[24px] leading-[1.1] tracking-[-0.03em]',
              INK.ink,
            )}
          >
            Let neo take the clicks.
          </h3>
          <p className={cn('m-0 max-w-[46ch] text-[14.5px]', INK.ink2)}>
            Give your AI agent a task on a website. Your numbers show up here
            after the first run.
          </p>
        </div>
        <div className="flex flex-col gap-2.5">
          <div
            className={cn(
              'flex @2xl:flex-row flex-col items-start @2xl:items-center gap-2.5 rounded-xl border bg-[rgb(238_243_255/0.05)] py-3 pr-3 pl-3.5',
              INK.line,
            )}
          >
            <span
              className={cn('flex-1 text-[13.5px] leading-[1.45]', INK.ink)}
            >
              {TRY_TASK}
            </span>
            <button
              type="button"
              onClick={copy}
              className="inline-flex h-[34px] shrink-0 items-center gap-1.5 rounded-lg bg-[#eef3ff] px-3.5 font-semibold text-[#0c2742] text-[13.5px] hover:bg-white active:translate-y-px"
            >
              <Copy aria-hidden className="size-3.5" />
              {copied ? 'Copied' : 'Copy'}
            </button>
          </div>
          <p className={cn('m-0 text-[13px]', INK.ink3)}>
            Paste it into Claude Code, Codex or Cursor.
          </p>
        </div>
      </div>
    </Panel>
  )
}

function Loading({ className }: { className?: string }) {
  const block = 'block rounded-md bg-[rgb(238_243_255/0.09)]'
  return (
    <Panel className={className} label="Loading your ultrafast savings" busy>
      <div className="mb-3.5">
        <UltrafastWordmark className={WORDMARK} />
      </div>
      <div className="grid @2xl:grid-cols-[minmax(0,1.1fr)_minmax(0,1fr)] grid-cols-1 items-end gap-x-12 gap-y-[22px]">
        <span className={cn(block, 'h-[58px] w-[240px] max-w-full')} />
        <div className="grid grid-cols-2 gap-x-8 gap-y-5">
          <span className={cn(block, 'h-10')} />
          <span className={cn(block, 'h-10')} />
          <span className={cn(block, 'h-10')} />
          <span className={cn(block, 'h-10')} />
        </div>
      </div>
    </Panel>
  )
}
