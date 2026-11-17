/** Sorting the run table by one of its columns. A run still running has no time or reply yet. */

import type { RunSummary } from '../../shared/api'
import { neoMs } from './measures'

export type SortKey = 'seq' | 'neo' | 'roundTrip' | 'code' | 'reply'
export type SortDirection = 'asc' | 'desc'

export interface RunSort {
  key: SortKey
  direction: SortDirection
}

export const DEFAULT_SORT: RunSort = { key: 'seq', direction: 'asc' }

function value(run: RunSummary, key: SortKey): number | null {
  switch (key) {
    case 'seq':
      return run.seq
    case 'neo':
      return neoMs(run)
    case 'roundTrip':
      return run.roundTripMs
    case 'code':
      return run.codeSize.tokens
    case 'reply':
      return run.replySize?.tokens ?? null
  }
}

/**
 * The runs in the chosen order. Runs with no value for the column (still running, or no reply)
 * go last whichever the direction, and ties keep the order the calls arrived in.
 */
export function sortRuns(runs: RunSummary[], sort: RunSort): RunSummary[] {
  const sign = sort.direction === 'asc' ? 1 : -1
  return [...runs].sort((a, b) => {
    const left = value(a, sort.key)
    const right = value(b, sort.key)
    if (left === null || right === null) {
      if (left !== right) return left === null ? 1 : -1
      return a.seq - b.seq
    }
    return (left - right) * sign || a.seq - b.seq
  })
}

/**
 * The sort after a header is clicked: the same column flips direction; a new column starts with
 * the largest first ("which call was expensive"), except `#`, which starts in call order.
 */
export function nextSort(current: RunSort, key: SortKey): RunSort {
  if (current.key === key) {
    return { key, direction: current.direction === 'asc' ? 'desc' : 'asc' }
  }
  return { key, direction: key === 'seq' ? 'asc' : 'desc' }
}
