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

/**
 * Neo's own bookkeeping, which no line of the script calls: the snapshot baseline's two
 * operations (the prelude records the text a snapshot returned, and reads the stored one for a
 * delta) and the event pump's. Named, because Neo records them as ordinary built-ins; its
 * `neo_only` rows (neo-playwright's op table) also hold Neo's own versions of script calls,
 * such as `context.pages`, so that mark would say too much.
 */
const NEO_BOOKKEEPING = new Set([
  'page.snapshotPending',
  'page.snapshotBaseline',
  'events.listen',
  'events.next',
])

export function isNeoInternal(operation: NeoOperation): boolean {
  return NEO_BOOKKEEPING.has(operation.name)
}

/**
 * Where a call sits: its display line (null for one not on a line) and, among the calls made
 * from the same place in the script, which one it is. A line in a loop runs again for each pass,
 * so its second `context.newPage` is call 2 of 3; two calls written side by side on one line
 * (`[await page.title(), await page.title()]`) are two places, each called once.
 */
export interface CallPlace {
  line: number | null
  call: number
  calls: number
}

export function callPlaces(placed: PlacedOperations): Map<string, CallPlace> {
  const places = new Map<string, CallPlace>()
  const number = (operations: NeoOperation[], line: number | null) => {
    const totals = new Map<string, number>()
    for (const operation of operations) {
      const key = callSite(operation)
      totals.set(key, (totals.get(key) ?? 0) + 1)
    }
    const seen = new Map<string, number>()
    for (const operation of operations) {
      const key = callSite(operation)
      const call = (seen.get(key) ?? 0) + 1
      seen.set(key, call)
      places.set(operation.id, { line, call, calls: totals.get(key) ?? 1 })
    }
  }
  for (const [line, operations] of placed.byLine) number(operations, line)
  number(placed.unplaced, null)
  return places
}

/**
 * The script place a call came from, which a loop pass calls again: by its line and column
 * alone, since one place can call a different method each pass (`locator[method]()`). Neo's
 * bookkeeping records the site of the call it serves (`page.snapshotPending` that of its
 * `ariaSnapshot()`), so it is counted on its own; a call with no site, by its name.
 */
function callSite(operation: NeoOperation): string {
  const site = operation.site
  if (!site) return `${operation.name}@none`
  const place = `${site.line}:${site.column}`
  return isNeoInternal(operation) ? `${operation.name}@${place}` : place
}

/** `2nd`, `3rd`, `11th`, `21st`. */
export function ordinal(n: number): string {
  const teen = n % 100 >= 11 && n % 100 <= 13
  const suffix = teen ? 'th' : (['th', 'st', 'nd', 'rd'][n % 10] ?? 'th')
  return `${n}${suffix}`
}

/** `line 7 · call 2 of 3`, `line 7`, `not on a line`. */
export function placeText(place: CallPlace): string {
  const where = place.line === null ? 'not on a line' : `line ${place.line}`
  return place.calls > 1
    ? `${where} · call ${place.call} of ${place.calls}`
    : where
}

/** The operation the right panel shows: which run, which operation, and where it sits. */
export interface OperationSelection {
  runId: string
  operationId: string
  place: CallPlace
}
