/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { PlanSavings, SessionSummary } from '@browseros/claw-api'
import type { UseQueryResult } from '@tanstack/react-query'
import { ArrowRight, Info, Play } from 'lucide-react'
import { Link, useLocation } from 'react-router'
import { UltrafastNote } from '@/components/audit/UltrafastMarks'
import {
  HOW_WE_COUNT,
  meterTokens,
  tokensSavedSinceUltrafast,
} from '@/components/cockpit/energy-meter.helpers'
import { Button } from '@/components/motion/button/base'
import {
  Tooltip,
  TooltipContent,
  TooltipTrigger,
} from '@/components/motion/tooltip'
import { Skeleton } from '@/components/ui/skeleton'
import { cn } from '@/lib/utils'
import {
  chipTokens,
  pagesLabel,
  pursueNote,
  taskWhen,
  ultrafastTime,
} from '@/screens/audit/audit.helpers'
import { savingsTime } from './pro.helpers'

/** What ultrafast has done on this computer, from the task history. */
export function SavingsHero({
  savings,
  onShowMe,
}: {
  savings: UseQueryResult<PlanSavings>
  onShowMe: () => void
}) {
  if (savings.isPending) {
    return (
      <section aria-label="What ultrafast has saved" aria-busy>
        <Skeleton className="h-24 w-56 rounded-xl" />
        <Skeleton className="mt-5 h-10 w-80 max-w-full rounded-lg" />
      </section>
    )
  }
  if (savings.isError) {
    return (
      <section
        aria-label="What ultrafast has saved"
        className="flex flex-wrap items-center gap-3"
      >
        <p role="alert" className="text-[14.5px] text-ink-2">
          Couldn’t load what ultrafast has saved.
        </p>
        <Button
          variant="secondary"
          size="sm"
          onClick={() => void savings.refetch()}
        >
          Try again
        </Button>
      </section>
    )
  }

  const { totals, tasks, latest } = savings.data
  // The same figure as Home's meter, so the two never show different numbers.
  const tokensSaved = tokensSavedSinceUltrafast(savings.data)
  if (tasks === 0 || tokensSaved === null) {
    return (
      <section
        aria-label="What ultrafast has saved"
        className="flex flex-col items-start gap-3.5"
      >
        <BigNumber value="0" label="tokens saved so far" muted />
        <p className="text-[16px] text-ink-2">
          Run a task with ultrafast to start.
        </p>
        <Button variant="secondary" onClick={onShowMe}>
          <Play aria-hidden className="size-4" />
          Show me how
        </Button>
      </section>
    )
  }

  return (
    <>
      <section
        aria-label="What ultrafast has saved"
        className="flex flex-col items-start gap-5"
      >
        <BigNumber
          value={meterTokens(tokensSaved)}
          label="tokens saved since you got ultrafast"
        />
        <dl className="flex flex-wrap gap-x-10 gap-y-3">
          <Total
            value={totals.pagesRead.toLocaleString('en-US')}
            label="pages neo accelerated"
          />
          <Total
            value={tasks.toLocaleString('en-US')}
            label={tasks === 1 ? 'ultrafast run' : 'ultrafast runs'}
          />
          <Total value={savingsTime(totals.durationMs)} label="neo working" />
        </dl>
      </section>
      <section
        aria-labelledby="ultrafast-latest"
        className="flex flex-col gap-3"
      >
        <div className="flex items-center justify-between gap-3">
          <h2
            id="ultrafast-latest"
            className="font-bold text-[19px] text-cyanotype-ink tracking-[-0.015em]"
          >
            Latest
          </h2>
          <Link
            to="/audit"
            className="inline-flex items-center gap-1 font-semibold text-[14px] text-accent-ink hover:underline"
          >
            See all
            <ArrowRight aria-hidden className="size-4" />
          </Link>
        </div>
        <ul className="flex flex-col border-border-2 border-t">
          {latest.map((task) => (
            <LatestRun key={task.sessionId} task={task} />
          ))}
        </ul>
      </section>
    </>
  )
}

// ---------------------------------------------------------------------------

function BigNumber({
  value,
  label,
  muted = false,
}: {
  value: string
  label: string
  muted?: boolean
}) {
  return (
    <div className="flex flex-col gap-1">
      <span
        className={cn(
          'font-extrabold text-[76px] tabular-nums leading-[0.95] tracking-[-0.05em] md:text-[104px]',
          muted ? 'text-ink-4' : 'text-accent',
        )}
      >
        {value}
      </span>
      <span className="inline-flex items-center gap-1.5 font-bold text-[20px] text-cyanotype-ink">
        {label}
        <Tooltip>
          <TooltipTrigger asChild>
            <button
              type="button"
              aria-label="How tokens saved is counted"
              className="grid size-6 place-items-center rounded-full text-ink-3 hover:text-ink-2 focus-visible:outline-2 focus-visible:outline-ring"
            >
              <Info aria-hidden className="size-4" />
            </button>
          </TooltipTrigger>
          <TooltipContent>{HOW_WE_COUNT}</TooltipContent>
        </Tooltip>
      </span>
    </div>
  )
}

function Total({ value, label }: { value: string; label: string }) {
  return (
    <div className="flex flex-col-reverse">
      <dt className="text-[13px] text-ink-3">{label}</dt>
      <dd className="font-extrabold text-[26px] text-cyanotype-ink tabular-nums tracking-[-0.02em]">
        {value}
      </dd>
    </div>
  )
}

function LatestRun({ task }: { task: SessionSummary }) {
  const location = useLocation()
  const run = task.ultrafast
  const pursue = task.pursue
  const note = pursue
    ? pursueNote(pursue)
    : run
      ? `neo ran it in ${ultrafastTime(run.durationMs)}`
      : null
  // What neo measured, or for a pursue run from before pages were counted, the steps it took.
  const recorded = run
    ? `${pagesLabel(run.pagesRead)}, ${chipTokens(run.tokensSaved)} tokens saved`
    : pursue && pursue.steps.length > 0
      ? pursue.steps.length === 1
        ? '1 step'
        : `${pursue.steps.length} steps`
      : null
  return (
    <li className="border-border-2 border-b">
      <Link
        to={`/audit/${encodeURIComponent(task.sessionId)}`}
        state={{ from: location.pathname }}
        className="grid grid-cols-1 items-center gap-x-3.5 gap-y-1 py-3.5 transition-colors hover:bg-bg-canvas focus-visible:bg-bg-canvas focus-visible:outline-none sm:grid-cols-[minmax(0,1fr)_auto_auto]"
      >
        <span className="flex min-w-0 flex-col">
          <span className="truncate font-semibold text-[14.5px] text-cyanotype-ink">
            {task.name}
          </span>
          <span className="text-[13px] text-ink-3">
            {taskWhen(task.startedAt, Date.now())}
          </span>
        </span>
        {note ? (
          <UltrafastNote className="justify-self-start">{note}</UltrafastNote>
        ) : null}
        {recorded ? (
          <span className="whitespace-nowrap text-[13.5px] text-ink-2">
            {recorded}
          </span>
        ) : null}
      </Link>
    </li>
  )
}
