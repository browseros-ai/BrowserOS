const BULLET = /^(\s*)- /
const MAX_LCS_CELLS = 4_000_000

export interface SnapshotDiff {
  /** Line-level diff or bounded change summary; empty when nothing changed. */
  text: string
  added: number
  removed: number
  changed: boolean
  /** Counts are changed-window sizes because the line-level comparison was skipped. */
  lineDiffSkipped?: true
  urlChanged?: true
  beforeUrl?: string
  afterUrl?: string
}

export interface DiffOptions {
  contextRadius?: number
}

export interface SnapshotObservation {
  text: string
  url?: string
}

interface TaggedLine {
  gutter: ' ' | '-' | '+'
  text: string
}

interface ChangedWindow {
  start: number
  beforeEnd: number
  afterEnd: number
}

/**
 * Computes a compact line diff between rendered snapshots, or a bounded summary when the changed
 * window is too large for LCS. Distant unchanged context is elided from line-level results.
 */
export function diffSnapshots(
  before: string,
  after: string,
  opts: DiffOptions = {},
): SnapshotDiff {
  if (before === after) {
    return { text: '', added: 0, removed: 0, changed: false }
  }

  const beforeLines = splitLines(before)
  const afterLines = splitLines(after)
  const { start, beforeEnd, afterEnd } = findChangedWindow(
    beforeLines,
    afterLines,
  )
  const removedWindowLines = beforeEnd - start
  const addedWindowLines = afterEnd - start

  if (exceedsLcsBudget(removedWindowLines, addedWindowLines)) {
    return {
      text: [
        `Snapshot changed substantially: ${beforeLines.length} lines before, ${afterLines.length} lines after.`,
        `Line-level diff skipped because the changed region exceeds the ${MAX_LCS_CELLS}-cell comparison limit.`,
      ].join('\n'),
      added: addedWindowLines,
      removed: removedWindowLines,
      changed: true,
      lineDiffSkipped: true,
    }
  }

  const tagged: TaggedLine[] = []
  for (let i = 0; i < start; i++) {
    tagged.push({ gutter: ' ', text: beforeLines[i] })
  }
  appendDiffLines(
    tagged,
    beforeLines.slice(start, beforeEnd),
    afterLines.slice(start, afterEnd),
  )
  for (let i = beforeEnd; i < beforeLines.length; i++) {
    tagged.push({ gutter: ' ', text: beforeLines[i] })
  }

  let added = 0
  let removed = 0
  for (const line of tagged) {
    if (line.gutter === '+') added++
    else if (line.gutter === '-') removed++
  }

  const body = collapse(tagged, opts.contextRadius ?? 3)
  return {
    text: `${body}\n${added} added, ${removed} removed`,
    added,
    removed,
    changed: true,
  }
}

/** Compares successive page observations, returning the full snapshot when navigation changed the URL. */
export function diffSnapshotObservations(
  before: SnapshotObservation | undefined,
  after: SnapshotObservation,
  opts: DiffOptions = {},
): SnapshotDiff {
  const beforeUrl = before?.url
  const afterUrl = after.url
  if (isKnownUrl(beforeUrl) && isKnownUrl(afterUrl) && beforeUrl !== afterUrl) {
    return {
      text: after.text,
      added: 0,
      removed: 0,
      changed: true,
      urlChanged: true,
      beforeUrl,
      afterUrl,
    }
  }

  const diff = diffSnapshots(before?.text ?? '', after.text, opts)
  if (isKnownUrl(afterUrl)) return { ...diff, afterUrl }
  return diff
}

function isKnownUrl(url: string | undefined): url is string {
  return url !== undefined && url !== '' && url !== 'unknown'
}

function splitLines(value: string): string[] {
  return value === '' ? [] : value.split('\n')
}

function findChangedWindow(before: string[], after: string[]): ChangedWindow {
  let start = 0
  while (
    start < before.length &&
    start < after.length &&
    before[start] === after[start]
  ) {
    start++
  }

  let beforeEnd = before.length
  let afterEnd = after.length
  while (
    beforeEnd > start &&
    afterEnd > start &&
    before[beforeEnd - 1] === after[afterEnd - 1]
  ) {
    beforeEnd--
    afterEnd--
  }

  return { start, beforeEnd, afterEnd }
}

function exceedsLcsBudget(beforeLength: number, afterLength: number): boolean {
  return (
    beforeLength > 0 &&
    afterLength > 0 &&
    beforeLength + 1 > MAX_LCS_CELLS / (afterLength + 1)
  )
}

function appendDiffLines(
  tagged: TaggedLine[],
  before: string[],
  after: string[],
): void {
  if (before.length === 0) {
    for (const text of after) tagged.push({ gutter: '+', text })
    return
  }
  if (after.length === 0) {
    for (const text of before) tagged.push({ gutter: '-', text })
    return
  }

  const table = buildLcsTable(before, after)
  let i = 0
  let j = 0

  while (i < before.length && j < after.length) {
    if (before[i] === after[j]) {
      tagged.push({ gutter: ' ', text: before[i] })
      i++
      j++
    } else if (table[i + 1][j] >= table[i][j + 1]) {
      tagged.push({ gutter: '-', text: before[i] })
      i++
    } else {
      tagged.push({ gutter: '+', text: after[j] })
      j++
    }
  }

  while (i < before.length) {
    tagged.push({ gutter: '-', text: before[i++] })
  }
  while (j < after.length) {
    tagged.push({ gutter: '+', text: after[j++] })
  }
}

function buildLcsTable(before: string[], after: string[]): number[][] {
  const table = Array.from({ length: before.length + 1 }, () =>
    new Array<number>(after.length + 1).fill(0),
  )

  for (let i = before.length - 1; i >= 0; i--) {
    for (let j = after.length - 1; j >= 0; j--) {
      table[i][j] =
        before[i] === after[j]
          ? table[i + 1][j + 1] + 1
          : Math.max(table[i + 1][j], table[i][j + 1])
    }
  }

  return table
}

/** Keep changed lines plus a `radius` window of context; elide gaps with `…`. */
function collapse(tagged: TaggedLine[], radius: number): string {
  const keep = new Array<boolean>(tagged.length).fill(false)
  for (let i = 0; i < tagged.length; i++) {
    if (tagged[i].gutter === ' ') continue
    const lo = Math.max(0, i - radius)
    const hi = Math.min(tagged.length - 1, i + radius)
    for (let j = lo; j <= hi; j++) keep[j] = true
  }

  const out: string[] = []
  let prev = -1
  for (let i = 0; i < tagged.length; i++) {
    if (!keep[i]) continue
    if (prev >= 0 && i - prev > 1) out.push('…')
    const { gutter, text } = tagged[i]
    out.push(`${gutter} ${text.replace(BULLET, '$1')}`)
    prev = i
  }
  return out.join('\n')
}
