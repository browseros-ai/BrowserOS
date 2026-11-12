/**
 * Placing Neo's operations on the script's lines, and their waterfall bars.
 *
 * Neo records the script line and column that called each operation (`NeoOperation.site`, from
 * the call site it parses when the script calls the SDK). Operations without a site are Neo's
 * own work for the run (opening the task's page, listening for events, reading frames) or calls
 * made inside another operation; they go in a group of their own rather than on a line. The
 * name is the one Neo recorded, which can differ from what the script wrote
 * (`page.textContent('h1')` is recorded as `locator.textContent`).
 *
 * Each bar is drawn inside the run's own time, Neo's record `endedAt − startedAt`, so a glance
 * down a block shows which line the run spent its time on.
 */

import type { NeoOperation, NeoRunRecord } from '../../shared/api'

export interface PlacedOperations {
  /** Display line (1-based) → the operations its code called, in start order. */
  byLine: Map<number, NeoOperation[]>
  /** No site, or a site outside the script. */
  unplaced: NeoOperation[]
  /** The time the bars are drawn in: the run's, or as far as its operations reach. */
  spanMs: number
}

/**
 * `lineOfSite` maps Neo's site (the script as sent) to a display line: the formatted layout's
 * (`shared/code-format.ts`), or the line itself.
 */
export function placeOperations(
  record: NeoRunRecord,
  lineCount: number,
  lineOfSite: (site: { line: number; column: number }) => number | null = (
    site,
  ) => site.line,
): PlacedOperations {
  const byLine = new Map<number, NeoOperation[]>()
  const unplaced: NeoOperation[] = []
  const ordered = [...record.operations].sort(
    (a, b) => a.startOffsetMs - b.startOffsetMs || a.seq - b.seq,
  )
  for (const operation of ordered) {
    const line = operation.site ? lineOfSite(operation.site) : null
    if (line === null || line < 1 || line > lineCount) {
      unplaced.push(operation)
      continue
    }
    const onLine = byLine.get(line)
    if (onLine) onLine.push(operation)
    else byLine.set(line, [operation])
  }
  return { byLine, unplaced, spanMs: spanOf(record) }
}

/**
 * The run's duration from its record; while the record is still open, the furthest an
 * operation reached, so bars stay inside the track.
 */
export function spanOf(record: NeoRunRecord): number {
  const reached = record.operations.reduce(
    (furthest, operation) =>
      Math.max(furthest, operation.startOffsetMs + (operation.durationMs ?? 0)),
    0,
  )
  return Math.max(record.durationMs ?? reached, reached, 1)
}

export interface WaterfallBar {
  /** Where the operation started, as a share of the run's time (0..1). */
  left: number
  /** How long it took, as a share; an unfinished operation runs to the end of the track. */
  width: number
  /** Not finished when the record was read. */
  open: boolean
}

export function waterfallBar(
  operation: NeoOperation,
  spanMs: number,
): WaterfallBar {
  const left = clamp(operation.startOffsetMs / spanMs)
  if (operation.durationMs === null) {
    return { left, width: 1 - left, open: true }
  }
  return {
    left,
    width: Math.min(clamp(operation.durationMs / spanMs), 1 - left),
    open: false,
  }
}

function clamp(share: number): number {
  return Math.min(1, Math.max(0, Number.isFinite(share) ? share : 0))
}

/** `events.listen ×2, page.frames, context.newPage`: a group's operations by name, in order. */
export function namesWithCounts(operations: NeoOperation[]): string {
  const counts = new Map<string, number>()
  for (const operation of operations) {
    counts.set(operation.name, (counts.get(operation.name) ?? 0) + 1)
  }
  return [...counts]
    .map(([name, count]) => (count > 1 ? `${name} ×${count}` : name))
    .join(', ')
}
