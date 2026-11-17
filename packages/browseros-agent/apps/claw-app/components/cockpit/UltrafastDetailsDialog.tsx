/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The ultrafast card's Details, for ultrafast customers only: what neo saved since they got
 * ultrafast and over the last 30 and 7 days, on the same ink as the card. Free installs keep
 * SavingsCard's dialog.
 */

import type {
  PlanSavings,
  PlanSavingsMeter,
  PlanSavingsPeriod,
} from '@browseros/claw-api'
import { X } from 'lucide-react'
import { type ReactElement, useState } from 'react'
import { UltrafastWordmark } from '@/components/audit/UltrafastMarks'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
  CenterMorphModalTrigger,
} from '@/components/motion/center-morph-modal'
import { cn } from '@/lib/utils'
import { ultrafastTime } from '@/screens/audit/audit.helpers'
import { tokensSavedSinceUltrafast } from './energy-meter.helpers'
import { BigNumber, INK, InkFact } from './InkPanel'
import { timeSavedAmount } from './savings.helpers'
import {
  type AxisLabel,
  axisLabels,
  barPx,
  DETAILS_HOW_WE_COUNT,
  monthDay,
  PERIODS,
  type PeriodKey,
  type RunsColumn,
  type RunsStrip,
  runsStrip,
  stripCaption,
  stripPx,
} from './ultrafast-details.helpers'

/** The strip's width in the 560px dialog, and in the sheet on a 390px screen. */
const WIDE = 504
const NARROW = 354

export function UltrafastDetailsDialog({
  savings,
  trigger,
}: {
  savings: PlanSavings
  trigger: ReactElement
}) {
  const meter = savings.meter
  if (!meter) return null
  return (
    <CenterMorphModal>
      <CenterMorphModalTrigger>{trigger}</CenterMorphModalTrigger>
      <CenterMorphModalContent
        ariaLabel="Your savings"
        showCloseButton={false}
        backdropClassName="bg-[rgb(1_18_63/0.36)] backdrop-blur-none dark:bg-black/60"
        className={cn(
          'flex max-h-[calc(100dvh-96px)] max-w-[560px] flex-col border-0 shadow-[0_30px_60px_-24px_rgb(1_18_63/0.55)] dark:shadow-[inset_0_0_0_1px_rgb(238_243_255/0.08)]',
          INK.panel,
          // Below sm it docks to the bottom as a sheet. The modal's layer sits 16px inside the window and is the containing block here.
          'max-sm:fixed max-sm:-inset-x-4 max-sm:top-auto max-sm:-bottom-4 max-sm:max-h-[calc(100dvh-56px)] max-sm:w-auto max-sm:max-w-none',
        )}
      >
        <Report savings={savings} meter={meter} />
      </CenterMorphModalContent>
    </CenterMorphModal>
  )
}

function Report({
  savings,
  meter,
}: {
  savings: PlanSavings
  meter: PlanSavingsMeter
}) {
  const [key, setKey] = useState<PeriodKey>('since')
  const period = meter.periods[key]
  // The since tab shows the card's own figure, read the same way the card and the ultrafast page read it.
  const tokens =
    key === 'since'
      ? (tokensSavedSinceUltrafast(savings) ?? period.tokensSaved)
      : period.tokensSaved
  const where =
    key === 'since'
      ? `since ${monthDay(meter.since)}`
      : `in the last ${key === 'last30Days' ? '30' : '7'} days`

  return (
    <>
      <div className="relative flex-none px-[18px] pt-2 sm:px-7 sm:pt-5">
        <span
          aria-hidden
          className="mx-auto mb-3 block h-1 w-9 rounded-sm bg-[rgb(238_243_255/0.32)] sm:hidden"
        />
        <CenterMorphModalClose>
          <button
            type="button"
            aria-label="Close"
            className={cn(
              'absolute top-5 right-2.5 grid size-8 place-items-center rounded-lg hover:bg-[rgb(238_243_255/0.08)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#9cc0ff] sm:top-3.5 sm:right-3.5',
              INK.ink2,
            )}
          >
            <X aria-hidden className="size-4" />
          </button>
        </CenterMorphModalClose>
        <h2 className="m-0 flex items-baseline gap-2.5 pr-11 font-normal">
          <UltrafastWordmark className={cn('text-[16px]', INK.ink)} />{' '}
          <span className={cn('text-[13.5px]', INK.ink3)}>Your savings</span>
        </h2>
        <div
          role="tablist"
          aria-label="Period"
          className={cn(
            'mt-3.5 flex gap-[18px] overflow-x-auto border-b [scrollbar-width:none] sm:gap-[22px]',
            INK.line,
          )}
        >
          {PERIODS.map((tab) => (
            <button
              key={tab.key}
              type="button"
              role="tab"
              aria-selected={tab.key === key}
              onClick={() => setKey(tab.key)}
              className={cn(
                '-mb-px h-10 shrink-0 whitespace-nowrap border-b-2 text-[13.5px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#9cc0ff]',
                tab.key === key
                  ? 'border-[#eef3ff] font-semibold text-[#eef3ff]'
                  : cn('border-transparent font-medium', INK.ink3),
              )}
            >
              {tab.label}
            </button>
          ))}
        </div>
      </div>
      <div className="flex min-h-0 flex-1 flex-col overflow-y-auto overscroll-contain px-[18px] pt-[18px] pb-7 sm:px-7 sm:pt-[22px] sm:pb-[26px]">
        <div className="flex flex-col gap-2">
          <BigNumber tokens={tokens} className="text-[50px] sm:text-[60px]" />
          <span className={cn('text-[14.5px]', INK.ink2)}>
            tokens saved {where}
          </span>
        </div>
        {period.runs === 0 ? (
          <>
            <Rule />
            <p className={cn('m-0 text-[13.5px]', INK.ink2)}>
              {key === 'since'
                ? 'Your first ultrafast run will show here.'
                : `No ultrafast runs ${where}. Switch to "Since you got ultrafast" to see your earlier runs.`}
            </p>
          </>
        ) : (
          <>
            <dl className="m-0 mt-6 grid grid-cols-2 gap-x-6 gap-y-[18px] sm:grid-cols-3 sm:gap-x-8 sm:gap-y-5">
              <InkFact
                label="of your time saved"
                value={timeSavedAmount(period.timeSavedMs)}
              />
              <InkFact
                label={period.runs === 1 ? 'ultrafast run' : 'ultrafast runs'}
                value={period.runs.toLocaleString('en-US')}
              />
              <InkFact
                label="steps done by neo"
                value={
                  period.neoStepShare == null
                    ? 'None yet'
                    : `${period.neoStepShare}%`
                }
              />
              <InkFact
                label="fastest run"
                value={
                  period.fastestRunMs == null
                    ? 'None yet'
                    : ultrafastTime(period.fastestRunMs)
                }
              />
              <InkFact
                label="average run"
                value={ultrafastTime(period.durationMs / period.runs)}
              />
              <InkFact
                label="pages neo accelerated"
                value={period.pagesRead.toLocaleString('en-US')}
              />
            </dl>
            <Rule />
            <Runs
              periodKey={key}
              period={period}
              since={meter.since}
              now={Date.now()}
            />
          </>
        )}
        <p className={cn('m-0 mt-5 text-[12.5px] leading-normal', INK.ink3)}>
          {DETAILS_HOW_WE_COUNT}
        </p>
      </div>
    </>
  )
}

function Rule() {
  return <hr className={cn('m-0 mt-[26px] mb-5 border-0 border-t', INK.line)} />
}

function Runs({
  periodKey,
  period,
  since,
  now,
}: {
  periodKey: PeriodKey
  period: PlanSavingsPeriod
  since: number
  now: number
}) {
  const strip = runsStrip(periodKey, period, since, now)
  const unit = strip.weekly ? 'week' : 'day'
  const busiestColumn = strip.columns.find(
    (column) => column.runs === strip.busiest,
  )
  return (
    <section>
      <h3
        className={cn(
          'm-0 mb-3 flex items-baseline justify-between gap-3 font-semibold text-[14px]',
          INK.ink,
        )}
      >
        Runs
        {period.siteTasks > 0 ? (
          <span className={cn('font-normal text-[13px]', INK.ink3)}>
            {period.siteTasksFinished.toLocaleString('en-US')} of{' '}
            {period.siteTasks.toLocaleString('en-US')} site tasks finished
          </span>
        ) : null}
      </h3>
      <div data-strip={strip.bars ? 'bars' : 'squares'}>
        <div
          role="img"
          aria-label={`${period.runs} ultrafast ${period.runs === 1 ? 'run' : 'runs'}, the busiest ${unit} ${busiestColumn?.label ?? ''} with ${strip.busiest}`}
          className="relative grid items-end"
          style={{
            gridTemplateColumns: `repeat(${strip.columns.length}, minmax(0, 1fr))`,
            height: stripPx(strip),
          }}
        >
          {strip.columns.map((column, index) => (
            <Column
              // biome-ignore lint/suspicious/noArrayIndexKey: columns are fixed positions in time
              key={index}
              column={column}
              strip={strip}
              edge={
                index === 0
                  ? 'start'
                  : index === strip.columns.length - 1
                    ? 'end'
                    : null
              }
            />
          ))}
        </div>
        <Axis
          labels={axisLabels(periodKey, strip, NARROW)}
          className="sm:hidden"
        />
        <Axis
          labels={axisLabels(periodKey, strip, WIDE)}
          className="max-sm:hidden"
        />
        <p className={cn('m-0 mt-2 text-[12.5px]', INK.ink3)}>
          {stripCaption(strip)}
        </p>
      </div>
    </section>
  )
}

function Column({
  column,
  strip,
  edge,
}: {
  column: RunsColumn
  strip: RunsStrip
  edge: 'start' | 'end' | null
}) {
  const tip = `${column.label}: ${
    column.before
      ? 'before ultrafast'
      : column.runs === 1
        ? '1 run'
        : `${column.runs} runs`
  }`
  return (
    <span
      data-tip={tip}
      data-runs={column.before ? undefined : column.runs}
      className={cn(
        'relative flex h-full flex-col-reverse items-center gap-0.5 border-b pb-1',
        column.before
          ? 'border-[rgb(238_243_255/0.1)] border-dotted'
          : INK.line,
        'hover:after:absolute hover:after:bottom-[calc(100%+4px)] hover:after:z-[3] hover:after:whitespace-nowrap hover:after:rounded-md hover:after:bg-[#eef3ff] hover:after:px-[7px] hover:after:py-[3px] hover:after:font-medium hover:after:text-[#0c2742] hover:after:text-[11.5px] hover:after:content-[attr(data-tip)]',
        edge === 'start'
          ? 'hover:after:left-0'
          : edge === 'end'
            ? 'hover:after:right-0'
            : 'hover:after:left-1/2 hover:after:-translate-x-1/2',
      )}
    >
      {column.purchase ? (
        <span
          aria-hidden
          className="absolute -bottom-[5px] left-1/2 -ml-[0.75px] h-[7px] w-[1.5px] bg-[#9cc0ff]"
        />
      ) : null}
      {column.before || column.runs === 0 ? null : strip.bars ? (
        <>
          <b
            className="block w-[min(8px,62%)] flex-none rounded-t-[1.5px] bg-[#9cc0ff]"
            style={{ height: barPx(column.runs, strip.busiest) }}
          />
          {column.runs === strip.busiest ? (
            <em
              className={cn(
                'mb-0.5 flex-none font-medium font-mono text-[10px] not-italic tabular-nums leading-none',
                INK.ink2,
              )}
            >
              {column.runs}
            </em>
          ) : null}
        </>
      ) : (
        Array.from({ length: column.runs }, (_, square) => (
          <i
            // biome-ignore lint/suspicious/noArrayIndexKey: one square per run
            key={square}
            data-square
            className="block size-[7px] flex-none rounded-[1.5px] bg-[#9cc0ff]"
          />
        ))
      )}
    </span>
  )
}

function Axis({
  labels,
  className,
}: {
  labels: AxisLabel[]
  className: string
}) {
  return (
    <div
      aria-hidden
      className={cn(
        'relative mt-1.5 h-3.5 font-mono text-[10.5px] leading-[14px]',
        INK.ink3,
        className,
      )}
    >
      {labels.map((label) => (
        <span
          key={`${label.text}-${label.left}`}
          className={cn(
            'absolute top-0 whitespace-nowrap',
            label.kind === 'purchase' && cn('font-medium', INK.accent),
            label.kind === 'before' && 'font-sans text-[11.5px]',
          )}
          style={{ left: `${(label.left * 100).toFixed(2)}%` }}
        >
          {label.text}
        </span>
      ))}
    </div>
  )
}
