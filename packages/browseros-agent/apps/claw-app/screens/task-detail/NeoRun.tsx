/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Check, Hand, HandHelping } from 'lucide-react'
import { useState } from 'react'
import type {
  DescribedStep,
  NeoRun as NeoRunRecord,
} from '@/lib/step-sentences'
import { cn } from '@/lib/utils'
import { chipTokens, ultrafastTime } from '@/screens/audit/audit.helpers'
import {
  offsetFromTakeover,
  offsetLabel,
  type RailItem,
  runLastIndex,
} from './task-detail.helpers'

/** A run with more steps than this folds to one row until opened. */
export const FOLD_AFTER = 6

/** Goals longer than this are clamped to two lines with Show all. */
const LONG_GOAL = 70

interface NeoRunProps {
  run: Extract<RailItem, { kind: 'run' }>
  steps: readonly DescribedStep[]
  activeIndex: number
  /** The run's ending is the chosen row. */
  endChosen: boolean
  /** The agent's name, for "handed the page back to Claude". */
  agent: string
  startedAt: number
  /** Nothing follows the run in the list. */
  last: boolean
  onSelect: (step: DescribedStep) => void
  onSelectEnd: () => void
}

/**
 * neo's run on the rail: the takeover with its goal, the steps neo took on a Signal Blue rail
 * timed from the takeover, and how the run ended.
 */
export function NeoRun({
  run,
  steps,
  activeIndex,
  endChosen,
  agent,
  startedAt,
  last,
  onSelect,
  onSelectEnd,
}: NeoRunProps) {
  const [opened, setOpened] = useState(false)
  const takeover = steps[run.takeover]
  const record = takeover?.neoRun
  if (!takeover || !record) return null

  const foldable = run.steps.length > FOLD_AFTER
  const playingInside = run.steps.includes(activeIndex)
  const open = !foldable || opened || playingInside
  const nextStep = steps[runLastIndex(run) + 1]

  return (
    <>
      <Takeover
        step={takeover}
        index={run.takeover}
        current={!endChosen && activeIndex === run.takeover}
        startedAt={startedAt}
        onSelect={onSelect}
      />
      {open
        ? run.steps.map((index) => {
            const step = steps[index]
            if (!step) return null
            const current = !endChosen && index === activeIndex
            return (
              <NeoStep
                key={step.dispatchIds[0] ?? index}
                step={step}
                index={index}
                current={current}
                played={index <= activeIndex}
                onSelect={onSelect}
              />
            )
          })
        : null}
      {foldable && (!open || (opened && !playingInside)) ? (
        <FoldRow
          open={open}
          count={run.steps.length}
          durationMs={record.durationMs}
          onToggle={() => setOpened(!opened)}
        />
      ) : null}
      {record.outcome ? (
        <RunEnd
          record={record}
          outcome={record.outcome}
          chosen={endChosen}
          agent={agent}
          carriedOnAt={
            nextStep ? offsetLabel(nextStep.firstAt, startedAt) : undefined
          }
          last={last}
          onSelect={onSelectEnd}
        />
      ) : null}
    </>
  )
}

/** Where neo took over: its mark, the goal it was given, and when. */
function Takeover({
  step,
  index,
  current,
  startedAt,
  onSelect,
}: {
  step: DescribedStep
  index: number
  current: boolean
  startedAt: number
  onSelect: (step: DescribedStep) => void
}) {
  const [goalOpen, setGoalOpen] = useState(false)
  const goal = step.neoRun?.goal
  return (
    <li className="relative">
      <span
        aria-hidden
        className="absolute top-[42px] bottom-0 left-[22px] z-[1] w-0.5 bg-primary"
      />
      <div
        className={cn(
          'grid grid-cols-[30px_minmax(0,1fr)_auto] items-start gap-3 rounded-md px-2 py-2.5 transition-colors duration-200',
          current ? 'bg-accent-tint' : 'hover:bg-bg-sunken',
        )}
      >
        <button
          type="button"
          aria-label="Play from where neo took over"
          onClick={() => onSelect(step)}
          className={cn(
            'relative z-[2] grid size-[30px] place-items-center rounded-full border-2 border-primary bg-card outline-none focus-visible:ring-2 focus-visible:ring-ring',
            current && 'ring-[3px] ring-accent-tint-2',
          )}
        >
          <img
            src="/icons/browserclaw.svg"
            alt=""
            draggable={false}
            className="size-4"
          />
        </button>
        <div className="min-w-0 pt-1">
          <button
            type="button"
            data-step-index={index}
            aria-current={current ? 'step' : undefined}
            onClick={() => onSelect(step)}
            className="block text-left font-semibold text-[14.5px] text-ink leading-[1.4] outline-none focus-visible:underline"
          >
            <span className="mr-1.5 font-extrabold text-[13.5px] text-accent-ink italic tracking-[-0.01em]">
              ultrafast
            </span>
            neo took over the clicking
          </button>
          {goal ? (
            <p
              className={cn(
                'mt-0.5 text-[13.5px] text-ink-2 leading-[1.45]',
                !goalOpen && 'line-clamp-2',
              )}
            >
              “{goal}”
            </p>
          ) : null}
          {goal && goal.length > LONG_GOAL ? (
            <button
              type="button"
              onClick={() => setGoalOpen(!goalOpen)}
              className="mt-0.5 font-semibold text-[12.5px] text-accent-ink"
            >
              {goalOpen ? 'Show less' : 'Show all'}
            </button>
          ) : null}
        </div>
        <span className="pt-1.5 text-[12.5px] text-ink-4 tabular-nums">
          {offsetLabel(step.firstAt, startedAt)}
        </span>
      </div>
    </li>
  )
}

/** One step neo took: a dot on the blue rail and how long after the takeover it came. */
function NeoStep({
  step,
  index,
  current,
  played,
  onSelect,
}: {
  step: DescribedStep
  index: number
  current: boolean
  played: boolean
  onSelect: (step: DescribedStep) => void
}) {
  return (
    <li className="relative">
      <RunLine />
      <button
        type="button"
        data-step-index={index}
        aria-current={current ? 'step' : undefined}
        onClick={() => onSelect(step)}
        className={cn(
          'grid w-full grid-cols-[30px_minmax(0,1fr)_auto] items-center gap-3 rounded-md px-2 py-1.5 text-left outline-none transition-colors duration-200 focus-visible:ring-2 focus-visible:ring-ring',
          current ? 'bg-accent-tint' : 'hover:bg-bg-sunken',
        )}
      >
        <span
          aria-hidden
          className={cn(
            'relative z-[2] size-2.5 justify-self-center rounded-full border-2 border-primary',
            played ? 'bg-primary' : 'bg-card',
            current && 'ring-[3px] ring-accent-tint-2',
          )}
        />
        <span
          className={cn(
            'text-[14px] leading-[1.4]',
            played ? 'text-ink' : 'text-ink-2',
          )}
        >
          {step.sentence}
        </span>
        <span className="font-mono text-[12px] text-accent-ink tabular-nums">
          {offsetFromTakeover(step.neoOffsetMs ?? 0)}
        </span>
      </button>
    </li>
  )
}

/** A long run folded to one row, or the control that folds it again. */
function FoldRow({
  open,
  count,
  durationMs,
  onToggle,
}: {
  open: boolean
  count: number
  durationMs?: number
  onToggle: () => void
}) {
  const took = durationMs != null ? ` in ${ultrafastTime(durationMs)}` : ''
  return (
    <li className="relative">
      <RunLine />
      <button
        type="button"
        aria-expanded={open}
        onClick={onToggle}
        className="flex w-full items-center rounded-md px-2 py-1.5 text-left font-semibold text-[13.5px] text-accent-ink outline-none hover:bg-bg-sunken focus-visible:ring-2 focus-visible:ring-ring"
      >
        <span className="grid w-[30px] shrink-0 place-items-center">
          <span
            aria-hidden
            className="relative z-[2] size-2.5 rounded-full border-2 border-primary bg-card"
          />
        </span>
        <span className="ml-3">
          {open ? `Fold neo’s ${count} steps` : `${count} steps by neo${took}`}
        </span>
        {open ? null : <span className="ml-auto">Show them</span>}
      </button>
    </li>
  )
}

/** How the run ended, what neo did in it, and when the agent carried on. */
function RunEnd({
  record,
  outcome,
  chosen,
  agent,
  carriedOnAt,
  last,
  onSelect,
}: {
  record: NeoRunRecord
  outcome: NonNullable<NeoRunRecord['outcome']>
  chosen: boolean
  agent: string
  carriedOnAt?: string
  last: boolean
  onSelect: () => void
}) {
  const ended =
    outcome === 'completed'
      ? 'neo finished it'
      : outcome === 'cancelled'
        ? 'neo was stopped'
        : `neo handed the page back to ${agent}`
  return (
    <li className="relative">
      <span
        aria-hidden
        className="absolute top-0 left-[22px] z-[1] h-2.5 w-0.5 bg-primary"
      />
      {last ? null : (
        <span
          aria-hidden
          className="absolute top-[42px] -bottom-2 left-[22px] z-[1] w-0.5 rounded-full bg-border"
        />
      )}
      <button
        type="button"
        aria-current={chosen ? 'step' : undefined}
        onClick={onSelect}
        className={cn(
          'grid w-full grid-cols-[30px_minmax(0,1fr)] items-start gap-3 rounded-md px-2 py-2.5 text-left outline-none transition-colors duration-200 focus-visible:ring-2 focus-visible:ring-ring',
          chosen ? 'bg-accent-tint' : 'hover:bg-bg-sunken',
        )}
      >
        <RunEndMark
          outcome={outcome}
          ringed={chosen && outcome === 'handed_back'}
        />
        <span className="@container flex min-w-0 flex-col pt-1">
          <span className="font-semibold text-[14.5px] text-ink leading-[1.4]">
            {ended}
          </span>
          <span className="mt-1.5 grid @[230px]:grid-cols-2 grid-cols-1 gap-x-[18px]">
            <LedgerLine label="Steps" value={String(record.stepCount)} />
            {record.durationMs != null ? (
              <LedgerLine
                label="Time"
                value={ultrafastTime(record.durationMs)}
              />
            ) : null}
            {record.pagesRead != null ? (
              <LedgerLine label="Pages read" value={String(record.pagesRead)} />
            ) : null}
            {record.pageTokens != null ? (
              <LedgerLine
                label="Tokens saved"
                value={chipTokens(record.pageTokens)}
              />
            ) : null}
          </span>
          {carriedOnAt ? (
            <span className="mt-1.5 text-[12.5px] text-ink-3">
              {agent.charAt(0).toUpperCase() + agent.slice(1)} carried on at{' '}
              {carriedOnAt}
            </span>
          ) : null}
        </span>
      </button>
    </li>
  )
}

// ---------------------------------------------------------------------------

/** The Signal Blue rail through neo's steps, drawn over a chosen row's tint. */
function RunLine() {
  return (
    <span
      aria-hidden
      className="absolute inset-y-0 left-[22px] z-[1] w-0.5 bg-primary"
    />
  )
}

function RunEndMark({
  outcome,
  ringed,
}: {
  outcome: 'completed' | 'handed_back' | 'cancelled'
  ringed: boolean
}) {
  const Icon =
    outcome === 'completed'
      ? Check
      : outcome === 'cancelled'
        ? Hand
        : HandHelping
  return (
    <span
      aria-hidden
      className={cn(
        'relative z-[2] grid size-[30px] place-items-center rounded-full',
        outcome === 'completed'
          ? 'bg-primary text-primary-foreground'
          : outcome === 'cancelled'
            ? 'border border-border bg-bg-sunken text-ink-2'
            : 'border-2 border-primary bg-card text-primary',
        ringed && 'ring-[3px] ring-accent-tint-2',
      )}
    >
      <Icon className="size-4" strokeWidth={1.75} />
    </span>
  )
}

/** "Steps ........ 3": a label, a dotted leader and a figure. */
function LedgerLine({ label, value }: { label: string; value: string }) {
  return (
    <span className="flex items-baseline gap-1.5 py-0.5">
      <span className="whitespace-nowrap text-[12.5px] text-ink-3">
        {label}
      </span>
      <span
        aria-hidden
        className="min-w-2 flex-1 -translate-y-[3px] border-border border-b border-dotted"
      />
      <span className="whitespace-nowrap font-medium text-[13px] text-ink tabular-nums">
        {value}
      </span>
    </span>
  )
}
