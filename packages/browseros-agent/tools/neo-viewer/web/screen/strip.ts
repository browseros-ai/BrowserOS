/**
 * The "where the time went" strip: the task's wall clock from its first call to its last reply
 * (or now, while a run is running), with each run drawn where it sat in it. A run spans its round
 * trip (sent → answered); the space between runs is the time between calls.
 *
 * Positions are shares of the strip's width (0..1), so the component draws them as percentages
 * at any width.
 */

import type { RunSummary } from '../../shared/api'
import { failed } from './measures'
import { clock } from './units'

export type StripTone = 'neo' | 'bad' | 'busy' | 'idle'

export interface StripBlock {
  runId: string
  seq: number
  left: number
  width: number
  tone: StripTone
  /** Whether the run's number is printed above it; dense strips print every few. */
  labelled: boolean
}

export interface StripTick {
  left: number
  label: string
}

export interface StripLayout {
  startMs: number
  endMs: number
  /** True while a run is running: the strip ends at now. */
  live: boolean
  blocks: StripBlock[]
  /** Wall-clock marks between the start label and the end label. */
  ticks: StripTick[]
  startLabel: string
  endLabel: string
}

/** Tick spacings to choose from, in seconds; the strip uses the smallest that gives ≤ 4 ticks. */
const TICK_STEPS_S = [
  1, 2, 5, 10, 15, 30, 60, 120, 300, 600, 900, 1800, 3600, 7200, 21600,
]
const MAX_TICKS = 4
/** Ticks this close to either end would print over the start or end label. */
const TICK_EDGE = 0.06
const TICK_END_EDGE = 0.92
/** Run numbers closer than this share of the width would overlap (≈14 px of a 1,200 px strip). */
const LABEL_GAP = 0.012

export function stripLayout(
  runs: RunSummary[],
  nowMs: number,
  selectedId: string | null = null,
): StripLayout | null {
  if (runs.length === 0) return null
  const live = runs.some((run) => run.status === 'running')
  const startMs = Math.min(...runs.map((run) => Date.parse(run.sentAt)))
  let endMs = live ? nowMs : startMs
  for (const run of runs) {
    const ended = run.answeredAt ? Date.parse(run.answeredAt) : null
    endMs = Math.max(endMs, ended ?? Date.parse(run.sentAt))
  }
  // A single instant still gets a width, so its one block is visible.
  const span = Math.max(endMs - startMs, 1)

  const ordered = [...runs].sort((a, b) => a.seq - b.seq)
  let lastLabel = Number.NEGATIVE_INFINITY
  const blocks = ordered.map((run, index): StripBlock => {
    const sent = Date.parse(run.sentAt)
    const ended =
      run.status === 'running'
        ? nowMs
        : run.answeredAt
          ? Date.parse(run.answeredAt)
          : sent
    const left = (sent - startMs) / span
    const mustLabel = index === ordered.length - 1 || run.id === selectedId
    const labelled = mustLabel || left - lastLabel >= LABEL_GAP
    if (labelled) lastLabel = left
    return {
      runId: run.id,
      seq: run.seq,
      left,
      width: Math.max(0, ended - sent) / span,
      tone: toneOf(run),
      labelled,
    }
  })

  return {
    startMs,
    endMs,
    live,
    blocks,
    ticks: ticks(startMs, endMs),
    startLabel: clock(startMs),
    endLabel: live ? 'now' : clock(endMs),
  }
}

function toneOf(run: RunSummary): StripTone {
  if (run.status === 'running') return 'busy'
  if (failed(run)) return 'bad'
  if (run.status === 'ok') return 'neo'
  return 'idle'
}

/** Round wall-clock marks (`10:14:30`, `10:15:00`) inside the strip. */
export function ticks(startMs: number, endMs: number): StripTick[] {
  const span = endMs - startMs
  if (span <= 0) return []
  const stepS =
    TICK_STEPS_S.find((step) => span / (step * 1000) <= MAX_TICKS) ??
    (TICK_STEPS_S.at(-1) as number)
  const step = stepS * 1000
  const marks: StripTick[] = []
  for (let at = Math.ceil(startMs / step) * step; at < endMs; at += step) {
    const left = (at - startMs) / span
    if (left >= TICK_EDGE && left <= TICK_END_EDGE) {
      marks.push({ left, label: clock(at) })
    }
  }
  return marks
}
