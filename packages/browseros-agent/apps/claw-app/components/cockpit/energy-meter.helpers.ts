/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { PlanSavings } from '@browseros/claw-api'

/** How the meter and the ultrafast page count tokens saved, word for word. */
export const HOW_WE_COUNT =
  'Tokens saved adds up everything neo kept off your agent since you got ultrafast: short page descriptions instead of screenshots, and pages neo read itself. The two are measured differently, so the total is an estimate.'

/**
 * The one tokens saved figure: everything neo kept off the agent since this computer got
 * ultrafast. Home's meter and the ultrafast page both read it here, so they never differ.
 * Null until ultrafast has been on for this computer.
 */
export function tokensSavedSinceUltrafast(savings: PlanSavings): number | null {
  return savings.meter?.tokensSaved ?? null
}

/** Total tokens saved since ultrafast that are marked on the scale. Past the last, every 2.5M is. */
export const MILESTONES = [
  10_000, 100_000, 250_000, 500_000, 1_000_000, 2_500_000,
] as const

const MILESTONE_STEP = 2_500_000

/** "640", "6.1k", "254k", "3.2M", "1M". */
export function meterTokens(tokens: number): string {
  const value = Math.max(0, Math.round(tokens))
  if (value < 1_000) return String(value)
  // Cut over where rounding would reach the next unit, so 9,960 reads "10k" and 999,600 "1M".
  if (value < 9_950) return `${(value / 1_000).toFixed(1)}k`
  if (value < 999_500) return `${Math.round(value / 1_000)}k`
  return `${(value / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`
}

/** The milestone the needle moves toward. */
export function nextMilestone(tokens: number): number {
  const next = MILESTONES.find((milestone) => milestone > tokens)
  if (next) return next
  return (Math.floor(tokens / MILESTONE_STEP) + 1) * MILESTONE_STEP
}

/** The highest milestone already passed, or null below the first. */
export function passedMilestone(tokens: number): number | null {
  if (tokens < MILESTONES[0]) return null
  const last = MILESTONES[MILESTONES.length - 1]
  if (tokens >= last)
    return Math.floor(tokens / MILESTONE_STEP) * MILESTONE_STEP
  return (
    [...MILESTONES].reverse().find((milestone) => milestone <= tokens) ?? null
  )
}

/** The line under the scale: how far the next milestone is, and the last one passed. */
export interface MeterNote {
  toGo: string
  next: string
  /** "You just passed 250k" until halfway to the next, then "Passed 250k"; the first run gets its own words. */
  passed: string | null
}

export function meterNote(tokens: number, runs: number): MeterNote {
  const next = nextMilestone(tokens)
  const passed = passedMilestone(tokens)
  const toGo = meterTokens(next - tokens)
  if (passed === null) {
    return {
      toGo,
      next: meterTokens(next),
      passed: runs === 1 ? 'Your first ultrafast run is in' : null,
    }
  }
  const fresh = tokens < passed + (next - passed) / 2
  return {
    toGo,
    next: meterTokens(next),
    passed: `${fresh ? 'You just passed' : 'Passed'} ${meterTokens(passed)}`,
  }
}

/** "254k" as the figure "254" and its unit "k", so the unit can be set smaller. */
export function meterParts(tokens: number): { figure: string; unit: string } {
  const shown = meterTokens(tokens)
  const unit = /[kM]$/.test(shown) ? shown.slice(-1) : ''
  return { figure: unit ? shown.slice(0, -1) : shown, unit }
}

const SCALE_FLOOR = 1_000

/** The scale runs from 1k to 2.5M, or to the next milestone once the total is past 2.5M. */
export function scaleTop(tokens: number): number {
  return Math.max(MILESTONES[MILESTONES.length - 1], nextMilestone(tokens))
}

/** Where a value sits on the log scale, from 0 at 1k to 1 at the top. */
export function scalePosition(value: number, top: number): number {
  if (value <= SCALE_FLOOR) return 0
  return Math.min(
    1,
    Math.log10(value / SCALE_FLOOR) / Math.log10(top / SCALE_FLOOR),
  )
}

/** Ticks ruled like a slide rule: 1 to 9 in every decade from 1k, longer at each power of ten. */
export function scaleTicks(top: number): { value: number; decade: boolean }[] {
  const ticks: { value: number; decade: boolean }[] = []
  for (let decade = SCALE_FLOOR; decade <= top; decade *= 10) {
    for (let step = 1; step <= 9 && step * decade <= top; step++) {
      ticks.push({ value: step * decade, decade: step === 1 })
    }
  }
  if (ticks[ticks.length - 1]?.value !== top) {
    ticks.push({ value: top, decade: true })
  }
  return ticks
}

export type MarkState = 'passed' | 'next' | 'ahead'

/** The milestones on the scale, each passed, the next one, or still ahead. */
export function scaleMarks(
  tokens: number,
  top: number,
): { value: number; state: MarkState }[] {
  const marks: number[] = [...MILESTONES]
  for (
    let value = MILESTONES[MILESTONES.length - 1] + MILESTONE_STEP;
    value <= top;
    value += MILESTONE_STEP
  ) {
    marks.push(value)
  }
  const next = nextMilestone(tokens)
  return marks.map((value) => ({
    value,
    state: value <= tokens ? 'passed' : value === next ? 'next' : 'ahead',
  }))
}

const READING_KEY = 'ultrafastMeterReading:v1'

/** The total the needle last showed on this computer, or null the first time. */
export function lastReading(): number | null {
  try {
    const stored = localStorage.getItem(READING_KEY)
    if (stored === null) return null
    const value = Number(stored)
    return Number.isFinite(value) ? value : null
  } catch {
    return null
  }
}

export function rememberReading(tokens: number): void {
  try {
    localStorage.setItem(READING_KEY, String(Math.round(tokens)))
  } catch {
    // Without browser storage the needle just starts where it ends next time.
  }
}
