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
import { X, Zap } from 'lucide-react'
import { type ReactElement, useState } from 'react'
import { Link, useLocation } from 'react-router'
import { SiteChip } from '@/components/favicon/SiteChip'
import {
  CenterMorphModal,
  CenterMorphModalClose,
  CenterMorphModalContent,
  CenterMorphModalTrigger,
} from '@/components/motion/center-morph-modal'
import { cn } from '@/lib/utils'
import { pursuePill, ultrafastTime } from '@/screens/audit/audit.helpers'
import { CountUp, INK_2, INK_3, Ring } from './EnergyMeter'
import { tokensSavedSinceUltrafast } from './energy-meter.helpers'
import { timeSavedAmount } from './savings.helpers'
import {
  chart,
  chartBarHeight,
  howWeCount,
  PERIODS,
  type PeriodKey,
  periodStart,
} from './ultrafast-details.helpers'

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
        className="em em-dialog max-w-[600px] border-0"
      >
        <span aria-hidden className="em-grain" />
        <CenterMorphModalClose>
          <button
            type="button"
            aria-label="Close"
            className={cn(
              'em-chip absolute top-4 right-4 z-[3] grid size-8 place-items-center rounded-full',
              INK_2,
            )}
          >
            <X aria-hidden className="size-4" />
          </button>
        </CenterMorphModalClose>
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

  return (
    <div className="@container flex flex-col gap-[18px] @md:p-6 px-[18px] py-5">
      <h2 className="m-0 pr-10 font-bold text-[20px] text-white tracking-[-0.015em]">
        Your savings
      </h2>
      <div
        role="tablist"
        aria-label="Period"
        className="em-tabs inline-flex w-max max-w-full gap-0.5 rounded-xl p-1"
      >
        {PERIODS.map((tab) => (
          <button
            key={tab.key}
            type="button"
            role="tab"
            aria-selected={tab.key === key}
            onClick={() => setKey(tab.key)}
            className={cn(
              'inline-flex h-[30px] items-center whitespace-nowrap rounded-lg @md:px-3 px-[9px] font-medium @md:text-[13.5px] text-[13px]',
              tab.key === key ? 'bg-white text-[#01123f]' : INK_2,
            )}
          >
            {tab.label}
          </button>
        ))}
      </div>
      {/* Keyed on the tab so the ring fills and the numbers count up again on each switch. */}
      <div
        key={key}
        className="grid @md:grid-cols-[auto_minmax(0,1fr)] grid-cols-1 items-center justify-items-center @md:justify-items-stretch gap-[22px]"
      >
        <Ring tokens={tokens} size="dialog" />
        <WhereNeoHelped
          period={period}
          note={
            neverRan && key === 'since'
              ? 'Sites show up here after your first ultrafast run.'
              : 'No ultrafast runs in this period.'
          }
        />
      </div>
      <Tiles periodKey={key} period={period} neverRan={neverRan} />
      {period.runs > 0 ? (
        <RunsChart
          periodKey={key}
          period={period}
          since={meter.since}
          now={now}
        />
      ) : null}
      {latest.length > 0 ? <LatestRuns tasks={latest} /> : null}
      <p className={cn('m-0 text-[12.5px] leading-normal', INK_3)}>
        {howWeCount(key, meter.since)}
      </p>
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
    <div className="flex w-full min-w-0 flex-col gap-2">
      <span className={cn('text-[13px]', INK_2)}>Where neo helped</span>
      {period.topSites.length === 0 ? (
        <span className={cn('text-[14px]', INK_2)}>{note}</span>
      ) : (
        <>
          <ul className="m-0 mt-0.5 flex list-none flex-col gap-[9px] p-0">
            {period.topSites.map((site) => (
              <li
                key={site.site}
                className="flex items-center gap-2.5 text-[14px] text-white"
              >
                <SiteChip site={site.site} />
                <span className="@md:w-[132px] w-[112px] truncate">
                  {site.site}
                </span>
                <span className="em-site-bar h-1.5 flex-1 overflow-hidden rounded-[3px]">
                  <i
                    className="block h-full rounded-[3px]"
                    style={{ width: `${(site.runs / busiest) * 100}%` }}
                  />
                </span>
                <span className={cn('w-[52px] text-right text-[13px]', INK_3)}>
                  {site.runs === 1 ? '1 run' : `${site.runs} runs`}
                </span>
              </li>
            ))}
          </ul>
          <div
            className={cn(
              'em-fold mt-1 flex items-baseline justify-between gap-3 pt-2.5 text-[13.5px]',
              INK_2,
            )}
          >
            <span>Pages neo accelerated</span>
            <b className="font-extrabold text-[18px] text-white tracking-[-0.02em]">
              <CountUp to={period.pagesRead} />
            </b>
          </div>
        </>
      )}
    </div>
  )
}

function Tiles({
  periodKey,
  period,
  neverRan,
}: {
  periodKey: PeriodKey
  period: PlanSavingsPeriod
  neverRan: boolean
}) {
  const time = (
    <Tile
      glow
      value={timeSavedAmount(period.timeSavedMs)}
      label="of your time saved"
    />
  )
  const tasks = (
    <Tile
      value={<CountUp to={period.tasksFinished} />}
      label="tasks finished"
    />
  )
  if (period.runs === 0) {
    const where =
      periodKey === 'since'
        ? 'since you got ultrafast'
        : `in the last ${periodKey === 'last30Days' ? '30' : '7'} days`
    return (
      <>
        <div className="grid grid-cols-2 gap-2.5">
          {time}
          {tasks}
        </div>
        <div className="em-tile flex flex-col gap-1.5 rounded-xl px-[18px] py-4">
          <span className="font-bold text-[15px] text-white">
            {neverRan
              ? 'Your first ultrafast run will show here'
              : `No ultrafast runs ${where}`}
          </span>
          <span className={cn('text-[13.5px]', INK_2)}>
            Give your agent a task on a website and neo takes the clicks and
            typing itself.
            {!neverRan && periodKey !== 'since'
              ? ' Switch to "Since you got ultrafast" to see your earlier runs.'
              : ''}
          </span>
        </div>
      </>
    )
  }
  const average = ultrafastTime(period.durationMs / period.runs)
  return (
    <div className="grid @md:grid-cols-3 grid-cols-2 gap-2.5">
      {time}
      {tasks}
      <Tile value={<CountUp to={period.runs} />} label="ultrafast runs" />
      <Tile
        value={
          period.siteTasks > 0
            ? `${period.siteTasksFinished} of ${period.siteTasks}`
            : 'None yet'
        }
        label={
          period.siteTasks > 0
            ? 'site tasks neo finished on its own'
            : 'site tasks with pursue'
        }
      />
      <Tile
        value={
          period.neoStepShare == null ? (
            'None yet'
          ) : (
            <>
              <CountUp to={period.neoStepShare} />%
            </>
          )
        }
        label={
          period.neoStepShare == null
            ? 'site tasks with pursue'
            : 'of the steps, done by neo'
        }
      />
      <Tile
        value={average}
        label={
          period.fastestRunMs == null
            ? 'average run'
            : `average run, ${ultrafastTime(period.fastestRunMs)} fastest`
        }
      />
    </div>
  )
}

function Tile({
  value,
  label,
  glow = false,
}: {
  value: React.ReactNode
  label: string
  glow?: boolean
}) {
  return (
    <div
      data-glow={glow ? '' : undefined}
      className="em-tile flex flex-col gap-0.5 rounded-xl px-3.5 py-3"
    >
      <span className="font-extrabold text-[24px] text-white tracking-[-0.03em]">
        {value}
      </span>
      <span className={cn('text-[12.5px]', INK_3)}>{label}</span>
    </div>
  )
}

function RunsChart({
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
  const { title, bars, note } = chart(periodKey, period, since, now)
  const busiest = Math.max(1, ...bars.map((bar) => bar.runs))
  const gap = bars.length > 9 ? 'gap-1' : 'gap-1.5'
  return (
    <div className="flex flex-col gap-1.5">
      <span className={cn('text-[12.5px]', INK_3)}>{title}</span>
      <div
        role="img"
        aria-label={`${title}: ${bars.map((bar) => bar.runs).join(', ')}`}
        className={cn('flex h-[34px] items-end', gap)}
      >
        {bars.map((bar, index) => (
          <span
            // biome-ignore lint/suspicious/noArrayIndexKey: bars are fixed positions in time
            key={index}
            data-hit={bar.runs > 0 ? '' : undefined}
            data-purchase={bar.purchase ? '' : undefined}
            className="em-bar flex-1"
            style={
              {
                height: chartBarHeight(bar.runs, busiest),
                '--i': index,
              } as React.CSSProperties
            }
          />
        ))}
      </div>
      <div className={cn('flex text-[11px]', gap, INK_3)}>
        {bars.map((bar, index) => (
          // biome-ignore lint/suspicious/noArrayIndexKey: labels follow the bars
          <span key={index} className="flex-1 text-center">
            {bar.label}
          </span>
        ))}
      </div>
      {note ? <span className={cn('text-[12.5px]', INK_3)}>{note}</span> : null}
    </div>
  )
}

function LatestRuns({ tasks }: { tasks: SessionSummary[] }) {
  const location = useLocation()
  return (
    <div>
      <span className={cn('font-bold text-[13px] tracking-[0.01em]', INK_2)}>
        Latest ultrafast runs
      </span>
      <ul className="m-0 mt-1.5 flex list-none flex-col p-0">
        {tasks.map((task) => {
          const pill = task.pursue
            ? pursuePill(task.pursue)
            : task.ultrafast
              ? `ultrafast · ${ultrafastTime(task.ultrafast.durationMs)}`
              : null
          return (
            <li key={task.sessionId} className="em-fold">
              <Link
                to={`/audit/${encodeURIComponent(task.sessionId)}`}
                state={{ from: location.pathname }}
                className="flex flex-wrap items-center gap-x-3 gap-y-1.5 py-2.5 no-underline"
              >
                {task.site ? <SiteChip site={task.site} /> : null}
                <span className="min-w-0 flex-1 truncate font-semibold text-[14px] text-white">
                  {task.name}
                </span>
                {pill ? (
                  <span className="em-pill inline-flex h-[22px] shrink-0 items-center gap-[5px] rounded-full border px-[9px] font-semibold text-[12px]">
                    <Zap aria-hidden className="size-3" />
                    {pill}
                  </span>
                ) : null}
              </Link>
            </li>
          )
        })}
      </ul>
    </div>
  )
}
