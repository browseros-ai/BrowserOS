/**
 * "Where the time went": the task's wall clock, the time spent answering run() calls against
 * the time between calls (the model thinking, the agent's other tools, any human wait).
 */

import { Fragment } from 'react'
import type { RunSummary } from '../../shared/api'
import type { RunTotals } from './measures'
import { type StripLayout, type StripTone, stripLayout } from './strip'
import { seconds } from './units'

const BLOCK: Record<StripTone, string> = {
  neo: 'bg-neo',
  bad: 'bg-bad',
  busy: 'bg-busy',
  idle: 'bg-ink-4',
}

const LABEL: Record<StripTone, string> = {
  neo: 'text-ink-2',
  bad: 'text-bad',
  busy: 'text-busy-ink',
  idle: 'text-ink-3',
}

export function TimeStrip({
  runs,
  totals,
  nowMs,
  selectedId,
}: {
  runs: RunSummary[]
  totals: RunTotals
  nowMs: number
  selectedId: string | null
}) {
  const layout = stripLayout(runs, nowMs, selectedId)
  if (!layout) return null
  return (
    <section
      aria-label="Where the time went"
      className="flex h-[78px] shrink-0 items-center gap-5 border-line border-b px-5"
    >
      <div className="flex w-44 shrink-0 flex-col gap-[3px]">
        <h2 className="font-semibold text-[13px] text-black leading-4">
          Where {seconds(layout.endMs - layout.startMs)} went
        </h2>
        <p
          className="text-ink-2 text-xs leading-4"
          title="Neo's time: each round trip less the time the viewer held the call. Between calls: from a reply to the next call."
        >
          <span className="font-code text-neo">{seconds(totals.neoMs)}</span> in
          Neo
          <br />
          <span className="font-code">{seconds(totals.gapMs)}</span> between
          calls
        </p>
      </div>
      <Clock layout={layout} selectedId={selectedId} />
    </section>
  )
}

function Clock({
  layout,
  selectedId,
}: {
  layout: StripLayout
  selectedId: string | null
}) {
  const percent = (share: number) => `${(share * 100).toFixed(3)}%`
  return (
    <div className="relative h-[52px] min-w-0 flex-1 font-code text-[11px] leading-[14px]">
      <div className="absolute inset-x-0 top-[18px] h-3.5 rounded-[2px] bg-line-2" />
      {layout.blocks.map((block) => (
        <Fragment key={block.runId}>
          {block.hold > 0 && (
            <div
              title={`run ${block.seq}: held by the viewer before forwarding`}
              className="absolute top-[18px] h-3.5 bg-control"
              style={{
                left: percent(block.left),
                width: percent(block.hold),
              }}
            />
          )}
          <div
            title={`run ${block.seq}`}
            className={`absolute top-[18px] h-3.5 ${BLOCK[block.tone]}`}
            style={{
              left: `min(${percent(block.left + block.hold)}, calc(100% - 2px))`,
              width: `max(2px, ${percent(block.width)})`,
            }}
          />
        </Fragment>
      ))}
      {layout.blocks
        .filter((block) => block.labelled)
        .map((block) => {
          const selected = block.runId === selectedId
          return (
            <span
              key={block.runId}
              className={`absolute top-0 ${selected ? 'font-semibold text-neo' : LABEL[block.tone]}`}
              style={{ left: `min(${percent(block.left)}, calc(100% - 1ch))` }}
            >
              {block.seq}
            </span>
          )
        })}
      <span className="absolute top-9 left-0 text-ink-3">
        {layout.startLabel}
      </span>
      {layout.ticks.map((tick) => (
        <span
          key={tick.label}
          className="absolute top-9 text-ink-3"
          style={{ left: percent(tick.left) }}
        >
          {tick.label}
        </span>
      ))}
      <span className="absolute top-9 right-0 text-ink-3">
        {layout.endLabel}
      </span>
    </div>
  )
}
