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

/**
 * The line under the total while it is fresh: "You just passed 250k" until halfway to the next
 * milestone, and "Your first ultrafast run is in" after the first run. Null the rest of the time.
 */
export function milestoneNote(tokens: number, runs: number): string | null {
  const passed = passedMilestone(tokens)
  if (passed === null) {
    return runs === 1 ? 'Your first ultrafast run is in' : null
  }
  const next = nextMilestone(tokens)
  return tokens < passed + (next - passed) / 2
    ? `You just passed ${meterTokens(passed)}`
    : null
}

/** "254k" as the figure "254" and its unit "k", so the unit can be set smaller. */
export function meterParts(tokens: number): { figure: string; unit: string } {
  const shown = meterTokens(tokens)
  const unit = /[kM]$/.test(shown) ? shown.slice(-1) : ''
  return { figure: unit ? shown.slice(0, -1) : shown, unit }
}
