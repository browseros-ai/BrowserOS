/**
 * Neo's health in the top bar, the one piece of chrome always in view (the time strip shows
 * nothing until a run arrives, and the sidebar's top is the live screen). It sits after the
 * container line, behind the same hairline: the few numbers that say whether the browser is
 * healthy right now, each a quiet label over a figure in the table's mono type, and how fresh
 * they are. Clicking it opens the Health view (`HealthView`) in place of the runs.
 *
 *   │ IN FLIGHT  RUN ERRORS  OP ERRORS  TABS  CRASHES
 *   │ 1          0%          13%        3     0        ● live
 *
 * Counts are since the last Clear, the run list's own frame (`TelemetryHealth.sinceClear`);
 * runs in flight and tabs are Neo's gauges now. With no telemetry (`health` null: an image older
 * than telemetry, or none arrived yet) it renders nothing, so the screen is as it always was.
 * Every figure keeps its slot whatever its value (labels are the wider line), and freshness comes
 * last, so a changing number never moves its neighbours.
 */

import { type Ref, useEffect, useState } from 'react'
import type { TelemetryHealth } from '../../shared/api'
import { count, plural } from './units'

/** Health this old still reads as live: Neo is scraped every 2 s and health sent at most 1/s. */
export const LIVE_FOR_MS = 5_000

export interface Freshness {
  /** `live`, `7 s ago`, or `no metrics` (no source has answered for 10 s, or none ever did). */
  text: string
  tone: 'live' | 'late' | 'none'
  /** Words for the tooltip: where the numbers come from and when. */
  where: string
}

export function freshness(health: TelemetryHealth, now: number): Freshness {
  if (health.source === null || health.observedAt === null) {
    return {
      text: 'no metrics',
      tone: 'none',
      where:
        'No metrics from Neo in the last 10 s: it stopped answering, or it sends spans only.',
    }
  }
  const age = Math.max(0, now - health.observedAt)
  const seconds = Math.round(age / 1000)
  const where =
    health.source === 'scrape'
      ? `Scraped from Neo's GET /v1/metrics ${seconds} s ago.`
      : `Pushed by Neo over OTLP ${seconds} s ago.`
  return age < LIVE_FOR_MS
    ? { text: 'live', tone: 'live', where }
    : { text: `${seconds} s ago`, tone: 'late', where }
}

/**
 * A failure rate as the bar shows it: `0%`, `<1%`, `13%`; `–` when nothing ran since the Clear
 * (or no source is live). `bad` when anything failed.
 */
export function errorRate(
  failed: number | undefined,
  total: number | undefined,
): { text: string; bad: boolean } {
  if (failed === undefined || total === undefined || total === 0)
    return { text: '–', bad: false }
  if (failed === 0) return { text: '0%', bad: false }
  const percent = (failed / total) * 100
  return { text: percent < 1 ? '<1%' : `${Math.round(percent)}%`, bad: true }
}

/** The time now, epoch ms, read again once a second while the caller is shown. */
export function useSecondClock(): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), 1000)
    return () => clearInterval(timer)
  }, [])
  return now
}

export function HealthLine({
  health,
  open,
  onToggle,
  now,
  buttonRef,
}: {
  health: TelemetryHealth
  /** The Health view is showing: the line reads as pressed. */
  open: boolean
  onToggle: () => void
  /** For tests; the line keeps its own one-second clock otherwise. */
  now?: number
  /** The line's button, so focus can come back to it when the Health view closes. */
  buttonRef?: Ref<HTMLButtonElement>
}) {
  const clock = useSecondClock()
  const fresh = freshness(health, now ?? clock)
  const since = health.sinceClear
  const runs = errorRate(since?.runErrors, since?.runs)
  const ops = errorRate(since?.opErrors, since?.ops)
  const crashes = since?.crashes
  return (
    <div className="flex h-7 shrink-0 items-center border-line border-l pl-3">
      {/* Named by its figures (no aria-label), so a screen reader reads them; the tooltip
          sentence is its description. */}
      <button
        ref={buttonRef}
        type="button"
        onClick={onToggle}
        aria-pressed={open}
        title={tooltip(health, fresh)}
        className={`flex items-end gap-3.5 rounded-md px-2 py-[3px] text-left ${open ? 'bg-neo-tint' : 'hover:bg-panel'}`}
      >
        <Readout label="In flight" value={figure(health.runsInFlight)} />
        <Readout label="Run errors" value={runs.text} bad={runs.bad} />
        <Readout label="Op errors" value={ops.text} bad={ops.bad} />
        <Readout label="Tabs" value={figure(health.tabsOpen)} />
        <Readout
          label="Crashes"
          value={figure(crashes ?? null)}
          bad={crashes !== undefined && crashes > 0}
        />
        <span
          data-freshness={fresh.tone}
          className={`flex items-center gap-1.5 whitespace-nowrap text-[11px] leading-4 ${fresh.tone === 'late' ? 'text-busy-ink' : 'text-ink-3'}`}
        >
          <span
            aria-hidden="true"
            className={`block size-1.5 rounded-full ${fresh.tone === 'live' ? 'bg-ok' : fresh.tone === 'late' ? 'bg-busy' : 'border border-ink-4'}`}
          />
          {fresh.text}
        </span>
      </button>
    </div>
  )
}

/** A label over its figure: the run block's column-label type over the table's mono figures. */
function Readout({
  label,
  value,
  bad = false,
}: {
  label: string
  value: string
  bad?: boolean
}) {
  return (
    <span className="flex flex-col">
      <span className="whitespace-nowrap text-[10px] text-ink-3 uppercase leading-3 tracking-wide">
        {label}
      </span>
      <span
        className={`font-code text-xs leading-4 ${value === '–' ? 'text-ink-4' : bad ? 'text-bad' : 'text-ink'}`}
      >
        {value}
      </span>
    </span>
  )
}

function figure(value: number | null): string {
  return value === null ? '–' : count(value)
}

/** The figures in words, with their frames and source: what a hover should answer. */
function tooltip(health: TelemetryHealth, fresh: Freshness): string {
  const since = health.sinceClear
  const counted = since
    ? `Since the last Clear: ${count(since.runErrors)} of ${plural(since.runs, 'run')} and ${count(since.opErrors)} of ${plural(since.ops, 'operation')} failed; ${since.crashes === 0 ? 'no renderer crashes' : plural(since.crashes, 'renderer crash', 'renderer crashes')}.`
    : null
  const now =
    health.runsInFlight !== null && health.tabsOpen !== null
      ? `Now: ${plural(health.runsInFlight, 'run')} in flight, ${plural(health.tabsOpen, 'tab')} open.`
      : null
  return [
    "Neo's health.",
    counted,
    now,
    fresh.where,
    'Click for the Health view.',
  ]
    .filter(Boolean)
    .join(' ')
}
