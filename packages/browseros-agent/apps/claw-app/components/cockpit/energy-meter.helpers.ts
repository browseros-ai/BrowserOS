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

/** The first milestone; after it they step 1, 2.5, 5 per power of ten: 25k, 50k, 100k ... 10M, 25M. */
const FIRST_MILESTONE = 10_000
const LADDER = [1, 2.5, 5] as const

/** Every milestone up to and including the first one above `tokens`. */
function milestonesThrough(tokens: number): number[] {
  const milestones: number[] = []
  for (let power = FIRST_MILESTONE; ; power *= 10) {
    for (const step of LADDER) {
      const milestone = step * power
      milestones.push(milestone)
      if (milestone > tokens) return milestones
    }
  }
}

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
  return milestonesThrough(tokens).at(-1) ?? FIRST_MILESTONE
}

/** Every milestone already passed, lowest first. */
export function passedMilestones(tokens: number): number[] {
  return milestonesThrough(tokens).filter((milestone) => milestone <= tokens)
}

/** The highest milestone already passed, or null below the first. */
export function passedMilestone(tokens: number): number | null {
  return passedMilestones(tokens).at(-1) ?? null
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

/** The stretch the scale shows: from the last milestone passed (or 0) to the next one. */
export interface ScaleSpan {
  from: number
  to: number
}

/** Each stretch fills the whole ruler, so a bigger total never gets a thinner slice of it. */
export function scaleSpan(tokens: number): ScaleSpan {
  return { from: passedMilestone(tokens) ?? 0, to: nextMilestone(tokens) }
}

/** Where a value sits on the stretch, from 0 at its start to 1 at the next milestone. */
export function scalePosition(value: number, span: ScaleSpan): number {
  return Math.min(1, Math.max(0, (value - span.from) / (span.to - span.from)))
}

/** A round tick spacing that rules the stretch into about 25 steps, every fifth one longer. */
function tickStep(span: ScaleSpan): number {
  const raw = (span.to - span.from) / 25
  const power = 10 ** Math.floor(Math.log10(raw))
  const step = [1, 2, 2.5, 5].find((nice) => nice * power >= raw) ?? 10
  return step * power
}

export function scaleTicks(
  span: ScaleSpan,
): { value: number; major: boolean }[] {
  const step = tickStep(span)
  const ticks: { value: number; major: boolean }[] = []
  for (
    let index = Math.ceil(span.from / step);
    index * step <= span.to;
    index++
  ) {
    ticks.push({ value: index * step, major: index % 5 === 0 })
  }
  return ticks
}

/** The milestones passed before the stretch starts, folded into the trail at the left. */
export function trailCount(tokens: number): number {
  return Math.max(0, passedMilestones(tokens).length - 1)
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
