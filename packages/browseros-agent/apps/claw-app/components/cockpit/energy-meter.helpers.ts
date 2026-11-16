/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

/** Total tokens saved since ultrafast that earn a trophy. Past the last, every 2.5M does. */
export const MILESTONES = [
  10_000, 100_000, 250_000, 500_000, 1_000_000, 2_500_000,
] as const

const MILESTONE_STEP = 2_500_000

/** "640", "6.1k", "254k", "3.2M", "1M". */
export function meterTokens(tokens: number): string {
  const value = Math.max(0, Math.round(tokens))
  if (value < 1_000) return String(value)
  if (value < 10_000) return `${(value / 1_000).toFixed(1)}k`
  if (value < 1_000_000) return `${Math.round(value / 1_000)}k`
  return `${(value / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`
}

/** The milestone the ring fills toward. */
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

/**
 * What the milestone card says. A trophy holds until the total is halfway to the next milestone,
 * then the card goes quiet and counts down to it. The first ultrafast run gets its own card.
 */
export type MilestoneCard =
  | { kind: 'first'; tokens: number; key: 'first' }
  | { kind: 'passed'; milestone: number; key: string }
  | { kind: 'quiet'; toGo: number; next: number }

export function milestoneCard(tokens: number, runs: number): MilestoneCard {
  const next = nextMilestone(tokens)
  const passed = passedMilestone(tokens)
  if (passed === null) {
    return runs === 1
      ? { kind: 'first', tokens, key: 'first' }
      : { kind: 'quiet', toGo: next - tokens, next }
  }
  if (tokens < passed + (next - passed) / 2) {
    return { kind: 'passed', milestone: passed, key: String(passed) }
  }
  return { kind: 'quiet', toGo: next - tokens, next }
}

export function milestoneCopy(card: MilestoneCard): {
  title: string
  line: string
} {
  if (card.kind === 'first') {
    return {
      title: 'Your first ultrafast run is in',
      line: `${meterTokens(card.tokens)} tokens saved since you got ultrafast. Next stop: 10k.`,
    }
  }
  if (card.kind === 'quiet') {
    return {
      title: `${meterTokens(card.toGo)} to go to ${meterTokens(card.next)}`,
      line: 'Every task neo helps your agent with moves this along.',
    }
  }
  const amount = meterTokens(card.milestone)
  if (card.milestone === 500_000) {
    return {
      title: 'Half a million tokens saved',
      line: `${amount} tokens your agent never had to spend since you got ultrafast.`,
    }
  }
  if (card.milestone === 1_000_000) {
    return {
      title: 'A million tokens saved',
      line: `${amount} tokens your agent never had to spend since you got ultrafast.`,
    }
  }
  return {
    title: `You passed ${amount} tokens saved`,
    line: `That's ${amount} tokens your agent never had to spend since you got ultrafast.`,
  }
}

/** ultrafast runs per local day for the 7 days ending today, oldest first. */
export function weekRuns(
  runTimes: readonly number[],
  now: number,
): { label: string; runs: number }[] {
  const today = new Date(now)
  today.setHours(0, 0, 0, 0)
  return Array.from({ length: 7 }, (_, index) => {
    const start = new Date(today)
    start.setDate(today.getDate() - (6 - index))
    const end = new Date(start)
    end.setDate(start.getDate() + 1)
    return {
      label: start.toLocaleDateString('en-US', { weekday: 'short' }),
      runs: runTimes.filter((at) => at >= start.getTime() && at < end.getTime())
        .length,
    }
  })
}

/** A bar's height: a stub for an empty day, taller per run, never past the top. */
export function barHeight(runs: number): string {
  return runs === 0 ? '14%' : `${Math.min(100, 30 + runs * 35)}%`
}

const SPARK_KEY = 'ultrafastMilestoneSeen:v1'

/** Whether this computer already celebrated the milestone. */
export function sparkSeen(key: string): boolean {
  try {
    return localStorage.getItem(SPARK_KEY) === key
  } catch {
    return true
  }
}

export function rememberSpark(key: string): void {
  try {
    localStorage.setItem(SPARK_KEY, key)
  } catch {
    // Without browser storage the burst just plays again next time.
  }
}
