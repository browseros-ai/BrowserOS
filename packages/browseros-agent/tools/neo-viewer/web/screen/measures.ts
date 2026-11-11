/**
 * The numbers behind the run table's bars and totals row. Every value is read from or summed over
 * the API's `RunSummary` fields; nothing is estimated here beyond the API's own chars ÷ 4.
 *
 * "Neo time" is the time Neo took to answer as the viewer measured it: the round trip less the
 * time the viewer held the call before forwarding it (`proxyMs`). Neo's own record
 * (`endedAt − startedAt`) joins the API with PR C; this is the stand-in until then.
 */

import type { RunSummary } from '../../shared/api'

export function neoMs(run: RunSummary): number | null {
  return run.roundTripMs === null
    ? null
    : Math.max(0, run.roundTripMs - run.proxyMs)
}

/** A value's share of the largest one, 0..1; bars are drawn as this share of their track. */
export function share(value: number, max: number): number {
  if (max <= 0 || value <= 0) return 0
  return Math.min(1, value / max)
}

/** What the model wrote plus what it read, the length of a run's stacked token bar. */
export function tokenTotal(run: RunSummary): number {
  return run.codeSize.tokens + (run.replySize?.tokens ?? 0)
}

/** The largest value in each bar column, so every row's bar shares one scale. */
export interface BarScale {
  neoMs: number
  tokens: number
}

export function barScale(runs: RunSummary[]): BarScale {
  let neo = 0
  let tokens = 0
  for (const run of runs) {
    neo = Math.max(neo, neoMs(run) ?? 0)
    tokens = Math.max(tokens, tokenTotal(run))
  }
  return { neoMs: neo, tokens }
}

/** One run's stacked token bar: the code part, then the reply part, as shares of the scale. */
export function tokenBar(
  run: RunSummary,
  scale: BarScale,
): { code: number; reply: number } {
  return {
    code: share(run.codeSize.tokens, scale.tokens),
    reply: share(run.replySize?.tokens ?? 0, scale.tokens),
  }
}

/** The totals row: sums over the rows shown, so the column adds up to what is on screen. */
export interface RunTotals {
  runs: number
  neoMs: number
  roundTripMs: number
  codeTokens: number
  replyTokens: number
  /** Time between a reply and the next call, summed (the agent's thinking, other tools, waits). */
  gapMs: number
}

export function runTotals(runs: RunSummary[]): RunTotals {
  const totals: RunTotals = {
    runs: runs.length,
    neoMs: 0,
    roundTripMs: 0,
    codeTokens: 0,
    replyTokens: 0,
    gapMs: 0,
  }
  for (const run of runs) {
    totals.neoMs += neoMs(run) ?? 0
    totals.roundTripMs += run.roundTripMs ?? 0
    totals.codeTokens += run.codeSize.tokens
    totals.replyTokens += run.replySize?.tokens ?? 0
    totals.gapMs += run.gapMs ?? 0
  }
  return totals
}

/** Whether a run ended without a reply the model could use. */
export function failed(run: RunSummary): boolean {
  return (
    run.status === 'error' ||
    run.status === 'rpc-error' ||
    run.status === 'http-error'
  )
}
