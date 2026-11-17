/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The energy meter's Details, for ultrafast customers only: what neo saved
 * since they got ultrafast and over the last 30 and 7 days. Free installs keep
 * SavingsCard's dialog.
 */

import type {
  PlanSavings,
  PlanSavingsMeter,
  PlanSavingsPeriod,
  SessionSummary,
} from '@browseros/claw-api'
import { X } from 'lucide-react'
import { type ReactElement, type ReactNode, useState } from 'react'
import { Link, useLocation } from 'react-router'
import { SiteChip } from '@/components/favicon/SiteChip'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
  CenterMorphModalTrigger,
} from '@/components/motion/center-morph-modal'
import { cn } from '@/lib/utils'
import { ultrafastNote, ultrafastTime } from '@/screens/audit/audit.helpers'
import { tokensSavedSinceUltrafast } from './energy-meter.helpers'
import { BigNumber, INK, Wordmark } from './InkPanel'
import { timeSavedAmount } from './savings.helpers'
import {
  type AxisLabel,
  axisLabels,
  barPx,
  countsFit,
  DETAILS_HOW_WE_COUNT,
  monthDay,
  PERIODS,
  type PeriodKey,
  periodStart,
  type RunsColumn,
  type RunsStrip,
  referenceStep,
  runsStrip,
  stripCaption,
  stripPx,
} from './ultrafast-details.helpers'

/** The strip's width in the 540px dialog, and in the sheet on a 390px screen. */
const WIDE = 500
const NARROW = 352

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
          'flex max-h-[calc(100dvh-96px)] max-w-[540px] flex-col border-border-2 bg-card shadow-[0_30px_60px_-24px_rgb(1_18_63/0.55)]',
          // Below sm it docks to the bottom as a sheet. The modal's layer sits 16px inside the window and is the containing block here.
          'max-sm:fixed max-sm:-inset-x-4 max-sm:top-auto max-sm:-bottom-4 max-sm:max-h-[calc(100dvh-56px)] max-sm:w-auto max-sm:max-w-none max-sm:border-b-0',
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
  const now = Date.now()
  const period = meter.periods[key]
  // The since tab shows the meter's own figure, read the same way the meter and the ultrafast page read it.
  const tokens =
    key === 'since'
      ? (tokensSavedSinceUltrafast(savings) ?? period.tokensSaved)
      : period.tokensSaved
  const neverRan = meter.periods.since.runs === 0
  const start = periodStart(key, meter.since, now)
  const latest = savings.latest
    .filter((task) => (task.endedAt ?? task.startedAt) >= start)
    .slice(0, 3)
  const caption =
    key === 'since'
      ? `tokens saved since ${monthDay(meter.since)}`
      : `tokens saved in the last ${key === 'last30Days' ? '30' : '7'} days`

  return (
    <>
      <div
        className={cn(
          'relative flex-none px-[18px] pt-2 sm:px-5 sm:pt-4',
          INK.panel,
        )}
      >
        <span
          aria-hidden
          className="mx-auto mb-2.5 block h-1 w-9 rounded-sm bg-[rgb(238_243_255/0.32)] sm:hidden"
        />
        <CenterMorphModalClose>
          <button
            type="button"
            aria-label="Close"
            className={cn(
              'absolute top-4 right-2.5 grid size-8 place-items-center rounded-lg hover:bg-[rgb(238_243_255/0.08)] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#9cc0ff] sm:top-2.5',
              INK.ink2,
            )}
          >
            <X aria-hidden className="size-4" />
          </button>
        </CenterMorphModalClose>
        <div className="flex items-baseline gap-2.5 pr-11">
          <h2
            className={cn(
              'm-0 font-bold text-[17px] tracking-[-0.015em]',
              INK.ink,
            )}
          >
            Your savings
          </h2>
          <Wordmark className={cn('text-[13.5px]', INK.ink2)} />
        </div>
        <div className="flex flex-wrap items-end justify-between gap-x-6 gap-y-2.5 pt-2.5 pb-3.5">
          <div>
            <BigNumber tokens={tokens} className="text-[40px] sm:text-[46px]" />
            <div className={cn('mt-1.5 text-[13.5px]', INK.ink2)}>
              {caption}
            </div>
          </div>
          <div className="flex">
            <Side label="of your time saved">
              {timeSavedAmount(period.timeSavedMs)}
            </Side>
            <Side label="tasks finished" divided>
              {period.tasksFinished}
            </Side>
          </div>
        </div>
        <div
          role="tablist"
          aria-label="Period"
          className={cn(
            'flex gap-[18px] overflow-x-auto border-t [scrollbar-width:none] sm:gap-[22px]',
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
                'h-10 shrink-0 whitespace-nowrap border-b-2 text-[13.5px] focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-[#9cc0ff]',
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
      <div className="flex min-h-0 flex-1 flex-col gap-5 overflow-y-auto overscroll-contain px-[18px] pt-3.5 pb-6 sm:px-5 sm:pt-4 sm:pb-5">
        <WhereNeoHelped
          period={period}
          note={
            neverRan && key === 'since'
              ? 'Sites show up here after your first ultrafast run.'
              : 'No ultrafast runs in this period.'
          }
        />
        <WhatNeoDid periodKey={key} period={period} neverRan={neverRan} />
        {period.runs > 0 ? (
          <Runs periodKey={key} period={period} since={meter.since} now={now} />
        ) : null}
        {latest.length > 0 ? <LatestRuns tasks={latest} /> : null}
        <p className="m-0 text-[12px] text-ink-3 leading-normal">
          {DETAILS_HOW_WE_COUNT}
        </p>
      </div>
    </>
  )
}

function Side({
  label,
  divided = false,
  children,
}: {
  label: string
  divided?: boolean
  children: ReactNode
}) {
  return (
    <div
      className={cn(
        'flex flex-col gap-px',
        divided ? cn('border-l pl-4', INK.line) : 'pr-4',
      )}
    >
      <b
        className={cn(
          'font-bold text-[18px] tabular-nums tracking-[-0.02em]',
          INK.ink,
        )}
      >
        {children}
      </b>
      <span className={cn('text-[12.5px]', INK.ink3)}>{label}</span>
    </div>
  )
}

function Heading({ children }: { children: ReactNode }) {
  return (
    <h3 className="m-0 mb-1.5 font-bold text-[13.5px] text-cyanotype-ink">
      {children}
    </h3>
  )
}

/** One ledger line on the light body: the label, dotted leaders, the figure. */
function LedgerRow({
  label,
  none = false,
  children,
}: {
  label: string
  none?: boolean
  children: ReactNode
}) {
  return (
    <div className="flex items-baseline gap-2 py-1">
      <dt className="flex min-w-0 flex-1 items-baseline gap-2 text-[13.5px] text-ink-2">
        <span className="whitespace-nowrap">{label}</span>
        <span
          aria-hidden
          className="min-w-3 flex-1 -translate-y-1 border-border border-b border-dotted"
        />
      </dt>
      <dd
        className={cn(
          'm-0 whitespace-nowrap',
          none
            ? 'font-medium text-[13.5px] text-ink-3'
            : 'font-bold text-[15px] text-cyanotype-ink tabular-nums',
        )}
      >
        {children}
      </dd>
    </div>
  )
}

function WhereNeoHelped({
  period,
  note,
}: {
  period: PlanSavingsPeriod
  note: string
}) {
  const busiest = Math.max(1, ...period.topSites.map((site) => site.runs))
  return (
    <section>
      <Heading>Where neo helped</Heading>
      {period.topSites.length === 0 ? (
        <p className="m-0 text-[13.5px] text-ink-2">{note}</p>
      ) : (
        <>
          <ul className="m-0 flex list-none flex-col gap-0.5 p-0">
            {period.topSites.map((site) => (
              <li
                key={site.site}
                className="grid grid-cols-[22px_104px_minmax(0,1fr)_56px] items-center gap-2.5 py-[3px] text-[13.5px] text-ink sm:grid-cols-[22px_140px_minmax(0,1fr)_64px] sm:gap-3"
              >
                <SiteChip site={site.site} />
                <span className="truncate">{site.site}</span>
                <span>
                  <span
                    className="block h-[5px] rounded-sm bg-primary"
                    style={{ width: `${(site.runs / busiest) * 100}%` }}
                  />
                </span>
                <span className="text-right text-[13px] text-ink-2 tabular-nums">
                  {site.runs === 1 ? '1 run' : `${site.runs} runs`}
                </span>
              </li>
            ))}
          </ul>
          <dl className="m-0">
            <LedgerRow label="Pages neo accelerated">
              {period.pagesRead}
            </LedgerRow>
          </dl>
        </>
      )}
    </section>
  )
}

function WhatNeoDid({
  periodKey,
  period,
  neverRan,
}: {
  periodKey: PeriodKey
  period: PlanSavingsPeriod
  neverRan: boolean
}) {
  if (period.runs === 0) {
    const where =
      periodKey === 'since'
        ? 'since you got ultrafast'
        : `in the last ${periodKey === 'last30Days' ? '30' : '7'} days`
    return (
      <section>
        <Heading>What neo did</Heading>
        <p className="m-0 font-semibold text-[14px] text-cyanotype-ink">
          {neverRan
            ? 'Your first ultrafast run will show here'
            : `No ultrafast runs ${where}`}
        </p>
        <p className="m-0 mt-1 text-[13.5px] text-ink-2">
          Give your agent a task on a website and neo takes the clicks and
          typing itself.
          {!neverRan && periodKey !== 'since'
            ? ' Switch to "Since you got ultrafast" to see your earlier runs.'
            : ''}
        </p>
      </section>
    )
  }
  return (
    <section>
      <Heading>What neo did</Heading>
      <dl className="m-0 grid grid-cols-1 sm:grid-cols-2 sm:gap-x-7">
        <LedgerRow label="ultrafast runs">{period.runs}</LedgerRow>
        <LedgerRow label="Fastest run" none={period.fastestRunMs == null}>
          {period.fastestRunMs == null
            ? 'None yet'
            : ultrafastTime(period.fastestRunMs)}
        </LedgerRow>
        <LedgerRow label="Site tasks finished" none={period.siteTasks === 0}>
          {period.siteTasks > 0
            ? `${period.siteTasksFinished} of ${period.siteTasks}`
            : 'None yet'}
        </LedgerRow>
        <LedgerRow label="Average run">
          {ultrafastTime(period.durationMs / period.runs)}
        </LedgerRow>
        <LedgerRow label="Steps done by neo" none={period.neoStepShare == null}>
          {period.neoStepShare == null ? 'None yet' : `${period.neoStepShare}%`}
        </LedgerRow>
        <LedgerRow label="Tasks finished">{period.tasksFinished}</LedgerRow>
      </dl>
    </section>
  )
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
  const room = {
    wide: countsFit(strip, WIDE),
    narrow: countsFit(strip, NARROW),
  }
  const step = strip.bars ? referenceStep(strip.busiest) : null
  const columns = {
    gridTemplateColumns: `repeat(${strip.columns.length}, minmax(0, 1fr))`,
  }
  const unit = strip.weekly ? 'week' : 'day'
  const busiestColumn = strip.columns.find(
    (column) => column.runs === strip.busiest,
  )
  return (
    <section>
      <Heading>Runs</Heading>
      <div
        data-strip={strip.bars ? 'bars' : 'squares'}
        className="flex flex-col gap-[5px]"
      >
        <div
          role="img"
          aria-label={`${period.runs} ultrafast ${period.runs === 1 ? 'run' : 'runs'}, the busiest ${unit} ${busiestColumn?.label ?? ''} with ${strip.busiest}`}
          className="relative grid items-end"
          style={{ ...columns, height: stripPx(strip) }}
        >
          {step ? (
            <div
              aria-hidden
              className={cn(
                'pointer-events-none absolute inset-x-0 border-border border-t border-dotted',
                room.narrow && 'max-sm:hidden',
                room.wide && 'sm:hidden',
              )}
              style={{ bottom: 3 + barPx(step, strip.busiest) }}
            >
              <b className="absolute -top-[7px] left-0 bg-card pr-1 font-medium font-mono text-[10px] text-ink-3 leading-3">
                {step}
              </b>
            </div>
          ) : null}
          {strip.columns.map((column, index) => (
            <Column
              // biome-ignore lint/suspicious/noArrayIndexKey: columns are fixed positions in time
              key={index}
              column={column}
              strip={strip}
              room={room}
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
        <p className="m-0 mt-px text-[12px] text-ink-3">
          {stripCaption(periodKey, strip, since)}
        </p>
      </div>
    </section>
  )
}

function Column({
  column,
  strip,
  room,
  edge,
}: {
  column: RunsColumn
  strip: RunsStrip
  room: { wide: boolean; narrow: boolean }
  edge: 'start' | 'end' | null
}) {
  const busiest = column.runs === strip.busiest
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
        'relative flex h-full flex-col-reverse items-center gap-0.5 border-border border-b pb-[3px]',
        column.before && 'border-dotted',
        'hover:after:absolute hover:after:bottom-[calc(100%+4px)] hover:after:z-[3] hover:after:whitespace-nowrap hover:after:rounded-md hover:after:bg-cyanotype-ink hover:after:px-[7px] hover:after:py-[3px] hover:after:font-medium hover:after:text-[11.5px] hover:after:text-background hover:after:content-[attr(data-tip)]',
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
          className="absolute -bottom-1 left-1/2 -ml-[0.75px] h-[7px] w-[1.5px] bg-primary"
        />
      ) : null}
      {column.before || column.runs === 0 ? null : strip.bars ? (
        <>
          <b
            className="block w-[min(8px,62%)] flex-none rounded-t-[1.5px] bg-primary"
            style={{ height: barPx(column.runs, strip.busiest) }}
          />
          <em
            className={cn(
              'mb-px flex-none font-medium font-mono text-[10px] text-ink-2 not-italic tabular-nums leading-none',
              !(room.narrow || busiest) && 'max-sm:hidden',
              !(room.wide || busiest) && 'sm:hidden',
            )}
          >
            {column.runs}
          </em>
        </>
      ) : (
        Array.from({ length: column.runs }, (_, square) => (
          <i
            // biome-ignore lint/suspicious/noArrayIndexKey: one square per run
            key={square}
            data-square
            className="block size-[7px] flex-none rounded-[1.5px] bg-primary"
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
        'relative h-[13px] font-mono text-[10.5px] text-ink-3 leading-[13px]',
        className,
      )}
    >
      {labels.map((label) => (
        <span
          key={`${label.text}-${label.left}`}
          className={cn(
            'absolute top-0 whitespace-nowrap',
            label.kind === 'purchase' && 'font-semibold text-accent-ink',
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

function LatestRuns({ tasks }: { tasks: SessionSummary[] }) {
  const location = useLocation()
  return (
    <section>
      <Heading>Latest ultrafast runs</Heading>
      <ul className="m-0 flex list-none flex-col gap-px p-0">
        {tasks.map((task) => {
          const note = ultrafastNote(task.pursue, task.ultrafast)
          return (
            <li key={task.sessionId}>
              <Link
                to={`/audit/${encodeURIComponent(task.sessionId)}`}
                state={{ from: location.pathname }}
                className="grid grid-cols-[22px_minmax(0,1fr)] items-center gap-x-3 gap-y-px py-1 text-[13.5px] no-underline sm:grid-cols-[22px_minmax(0,1fr)_auto]"
              >
                {task.site ? (
                  <SiteChip site={task.site} />
                ) : (
                  <span aria-hidden />
                )}
                <span className="truncate font-semibold text-cyanotype-ink">
                  {task.name}
                </span>
                {note ? (
                  <span className="col-start-2 truncate text-[13px] text-ink-3 sm:col-start-auto sm:max-w-[240px]">
                    {note}
                  </span>
                ) : null}
              </Link>
            </li>
          )
        })}
      </ul>
    </section>
  )
}
