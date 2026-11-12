/**
 * `page.ariaSnapshot({ delta: true })`: what changed since the last whole-page v1 snapshot of
 * the tab that the caller was given, as a line diff of the two texts.
 *
 *   frame.ts   reads the tab's stored baseline (`page.snapshotBaseline`: its text and how that
 *              text was rendered, kept by Rust on the host across runs), then records what the
 *              call gave as pending; the host stores it once the run ends with it in the
 *              caller's hands (neo-playwright's snapshot_baselines.rs and pending_baselines.rs)
 *   here       stored baseline + the snapshot's tree + the options the call passed
 *                → one of four first lines, then the hunks or the whole page; or, when the
 *                call's timeout passed before the baseline could be read, `NO_TIME` and the
 *                whole page
 *
 * The form rule (design section 7, "The form of a delta"): a baseline's form is how its text was
 * rendered (the `text` option and what the length rule chose, `urls`, `maxTokens`). A delta call
 * takes every option it leaves out from that form, and renders the new snapshot exactly as the
 * baseline was rendered, so the two texts differ only where the page does. An option it passes
 * that disagrees with the form means no comparison: two forms are never diffed, since every line
 * they render differently would be listed as a change that did not happen.
 *
 * Both texts are what the caller received (after the notice and the cap), so a delta only ever
 * prints lines the caller has seen or is seeing now. The notice line and the cut line are left
 * out of the comparison and of the result: they describe the snapshot's size, not the page.
 */
import {
  type PassedV1Options,
  type RenderedV1,
  renderSnapshotV1,
  renderSnapshotV1As,
  withDefaults,
} from './snapshot-v1'
import type {
  SnapshotBaseline,
  SnapshotForm,
  SnapshotResult,
  TextForm,
} from './wire'

export const NO_BASELINE = '# whole page: no earlier snapshot of this page'
export const NO_CHANGE = '# no change since your last snapshot of this tab'
/**
 * Not "the page changed": the same line answers a call whose options disagree with the
 * baseline's form, and it must be true either way.
 */
export const TOO_MUCH =
  '# whole page: too much differs from your last snapshot to list as changes'
/**
 * Not "no earlier snapshot": the call's timeout passed after its tree arrived and before it could
 * read the tab's stored baseline, so it cannot say whether there is one. True either way.
 */
export const NO_TIME =
  '# whole page: no time was left to look for an earlier snapshot of this page'

/** Unchanged lines printed on each side of a change. */
const CONTEXT = 3
/**
 * The most changed lines one diff looks for. The search keeps, for each edit count d it tries,
 * the 2d + 1 diagonals it reached, so this bounds its memory (about 4 MB at the limit) and its
 * time; a page that changed more than this is far past the half-of-the-output rule anyway, and
 * comes back whole.
 */
const MAX_CHANGED_LINES = 1000

/**
 * What a delta call returns, and the whole text, with its form, that becomes the tab's baseline
 * if the caller is given `returned`. After `# no change` or `# changes` that is the new snapshot
 * in the baseline's form, with the same form; after a whole page, that page with its own.
 */
export type DeltaResult = { returned: string; baseline: RenderedV1 }

/**
 * The delta call's answer, from the tab's stored baseline (`null`: none for this document), the
 * snapshot's tree and the v1 options the call passed.
 */
export function renderDelta(
  stored: SnapshotBaseline | null,
  result: SnapshotResult,
  passed: PassedV1Options,
): DeltaResult {
  if (stored === null) {
    const whole = renderSnapshotV1(result, withDefaults(passed))
    return { returned: `${NO_BASELINE}\n${whole.text}`, baseline: whole }
  }
  const { form } = stored
  if (agrees(passed, form)) {
    const current = renderSnapshotV1As(result, form)
    const changes = listChanges(stored.text, current)
    if (changes !== null)
      return { returned: changes, baseline: { text: current, form } }
  }
  // The whole page: what the call passed, the rest from the baseline's form. A `text` neither
  // call passed goes back to the length rule, since a whole page is a new snapshot and that rule
  // exists so a big page's text is not paid for unasked.
  const whole = renderSnapshotV1(result, {
    text: passed.text ?? passedText(form.text),
    urls: passed.urls ?? form.urls,
    maxTokens: passed.maxTokens ?? form.maxTokens,
  })
  return { returned: `${TOO_MUCH}\n${whole.text}`, baseline: whole }
}

/**
 * A delta call's answer when its timeout passed before the stored baseline could be read: the
 * whole page, rendered by the options the call passed as with no baseline, under `NO_TIME`. It
 * becomes no baseline, since the call that returns it records none.
 */
export function renderUncompared(
  result: SnapshotResult,
  passed: PassedV1Options,
): string {
  return `${NO_TIME}\n${renderSnapshotV1(result, withDefaults(passed)).text}`
}

/**
 * Whether every option the call passed agrees with `form`: `text: true` with a form that shows
 * the page's text, `text: false` with one that does not, `urls` and `maxTokens` when equal.
 */
function agrees(passed: PassedV1Options, form: SnapshotForm): boolean {
  return (
    (passed.text === undefined || passed.text === showsText(form.text)) &&
    (passed.urls === undefined || passed.urls === form.urls) &&
    (passed.maxTokens === undefined || passed.maxTokens === form.maxTokens)
  )
}

/** Whether a text of this form prints the page's text. */
function showsText(text: TextForm): boolean {
  return text === 'kept' || text === 'always'
}

/** The `text` option the call that rendered this form passed, `undefined` when it left it out. */
function passedText(text: TextForm): boolean | undefined {
  if (text === 'always') return true
  if (text === 'never') return false
  return undefined
}

/** The lines that describe the snapshot's size: the notice and the cut line (snapshot-cap.ts). */
function sizeLine(line: string): boolean {
  return line.startsWith('# text left out: ') || line.startsWith('# cut: ')
}

/**
 * The changes from `previous` to `current`, two texts rendered the same way: `NO_CHANGE`, or
 * the changes' first line and hunks; `null` when the changes would be longer than half of
 * `current`, too much to list.
 */
export function listChanges(previous: string, current: string): string | null {
  const before = previous.split('\n').filter((line) => !sizeLine(line))
  const after = current.split('\n').filter((line) => !sizeLine(line))
  const edits = diff(before, after, current.length)
  if (edits === null) return null
  if (edits.every((edit) => edit.kind === ' ')) return NO_CHANGE
  const changes = hunks(edits, before, after).join('\n')
  if (changes.length > current.length / 2) return null
  const added = edits.filter((edit) => edit.kind === '+').length
  const removed = edits.filter((edit) => edit.kind === '-').length
  return `# changes since your last snapshot of this tab: ${added} added, ${removed} removed\n${changes}`
}

/**
 * One line of the edit script. `before` and `after` are where the script stands in each text
 * at this line: the line's own index on its side, and on the other side the index of the next
 * line there.
 */
type Edit = { kind: ' ' | '-' | '+'; before: number; after: number }

/**
 * The shortest edit script from `before` to `after` (Myers' O(ND) diff), or `null` when it
 * would hold more changed lines than `MAX_CHANGED_LINES` or than can fit in half of `limit`
 * characters (each changed line costs at least its text, its marker and a newline). The common
 * head and tail are matched first, since a delta usually changes one place; a page that changed
 * nearly everywhere is caught by `unmatchedCost` before the search starts.
 */
function diff(
  before: readonly string[],
  after: readonly string[],
  limit: number,
): Edit[] | null {
  let head = 0
  while (
    head < before.length &&
    head < after.length &&
    before[head] === after[head]
  )
    head += 1
  let tail = 0
  while (
    tail < before.length - head &&
    tail < after.length - head &&
    before[before.length - 1 - tail] === after[after.length - 1 - tail]
  )
    tail += 1
  const n = before.length - head - tail
  const m = after.length - head - tail
  if (unmatchedCost(before, after, head, n, m) > limit / 2) return null
  let shortest = Number.POSITIVE_INFINITY
  for (let index = head; index < head + n; index++)
    shortest = Math.min(shortest, before[index].length)
  for (let index = head; index < head + m; index++)
    shortest = Math.min(shortest, after[index].length)
  const most = Math.min(
    MAX_CHANGED_LINES,
    n + m,
    n + m === 0 ? 0 : Math.floor(limit / 2 / (shortest + 2)),
  )
  const middle = middleScript(before, after, head, n, m, most)
  if (middle === null) return null
  const edits: Edit[] = []
  for (let index = 0; index < head; index++)
    edits.push({ kind: ' ', before: index, after: index })
  edits.push(...middle)
  for (let index = 0; index < tail; index++)
    edits.push({
      kind: ' ',
      before: head + n + index,
      after: head + m + index,
    })
  return edits
}

/**
 * A lower bound on the changes' length: a middle line with no equal partner left on the other
 * side is printed as a change whatever the script, at its length plus marker and newline. It
 * costs one pass, where the search would spend `MAX_CHANGED_LINES` rounds to learn the same.
 */
function unmatchedCost(
  before: readonly string[],
  after: readonly string[],
  offset: number,
  n: number,
  m: number,
): number {
  const spare = new Map<string, number>()
  for (let index = offset; index < offset + m; index++)
    spare.set(after[index], (spare.get(after[index]) ?? 0) + 1)
  let cost = 0
  for (let index = offset; index < offset + n; index++) {
    const left = spare.get(before[index]) ?? 0
    if (left > 0) spare.set(before[index], left - 1)
    else cost += before[index].length + 2
  }
  for (const [line, left] of spare) cost += left * (line.length + 2)
  return cost
}

/**
 * Myers' greedy search over the `n` by `m` middle starting at `offset` in both texts, keeping
 * each round's furthest reaches so the path can be walked back. `null` past `most` edits.
 * Round d touches only diagonals -d to d, so only those are kept: `rounds[d][k + d]`.
 */
function middleScript(
  before: readonly string[],
  after: readonly string[],
  offset: number,
  n: number,
  m: number,
  most: number,
): Edit[] | null {
  // reach[k + most + 1] is the furthest x on diagonal k = x - y.
  const width = 2 * most + 3
  const reach = new Int32Array(width)
  const rounds: Int32Array[] = []
  const at = (k: number) => k + most + 1
  for (let d = 0; d <= most; d++) {
    for (let k = -d; k <= d; k += 2) {
      // Down (an insertion) from diagonal k + 1, or right (a deletion) from k - 1.
      const down = k === -d || (k !== d && reach[at(k - 1)] < reach[at(k + 1)])
      let x = down ? reach[at(k + 1)] : reach[at(k - 1)] + 1
      let y = x - k
      while (x < n && y < m && before[offset + x] === after[offset + y]) {
        x += 1
        y += 1
      }
      reach[at(k)] = x
      if (x >= n && y >= m) {
        rounds.push(reach.slice(at(-d), at(d) + 1))
        return walkBack(rounds, n, m, offset)
      }
    }
    rounds.push(reach.slice(at(-d), at(d) + 1))
  }
  return null
}

/** The edit script, from the rounds' reaches, walked from the end back to the start. */
function walkBack(
  rounds: readonly Int32Array[],
  n: number,
  m: number,
  offset: number,
): Edit[] {
  const reversed: Edit[] = []
  let x = n
  let y = m
  for (let d = rounds.length - 1; d > 0; d--) {
    const k = x - y
    // Round d - 1 kept diagonals -(d - 1) to d - 1; a step into round d came from one of them.
    const previous = rounds[d - 1]
    const at = (diagonal: number) => diagonal + d - 1
    const down =
      k === -d || (k !== d && previous[at(k - 1)] < previous[at(k + 1)])
    const fromK = down ? k + 1 : k - 1
    const fromX = previous[at(fromK)]
    const fromY = fromX - fromK
    while (x > (down ? fromX : fromX + 1) && y > (down ? fromY + 1 : fromY)) {
      x -= 1
      y -= 1
      reversed.push({ kind: ' ', before: offset + x, after: offset + y })
    }
    if (down) {
      y -= 1
      reversed.push({ kind: '+', before: offset + x, after: offset + y })
    } else {
      x -= 1
      reversed.push({ kind: '-', before: offset + x, after: offset + y })
    }
  }
  // Round 0 is only the snake down the main diagonal.
  while (x > 0 && y > 0) {
    x -= 1
    y -= 1
    reversed.push({ kind: ' ', before: offset + x, after: offset + y })
  }
  return reversed.reverse()
}

/**
 * The changes in hunks: each one `@@` line naming the block it is in, then its lines with `+`,
 * `-` or a space glued in front, `CONTEXT` unchanged lines on each side. Changes whose context
 * would touch or overlap share one hunk.
 */
function hunks(
  edits: readonly Edit[],
  before: readonly string[],
  after: readonly string[],
): string[] {
  const out: string[] = []
  let index = 0
  while (index < edits.length) {
    if (edits[index].kind === ' ') {
      index += 1
      continue
    }
    const start = Math.max(0, index - CONTEXT)
    // Extend over changes until a run of more than twice the context separates the next one.
    let end = index
    let quiet = 0
    for (let next = index; next < edits.length; next++) {
      if (edits[next].kind === ' ') {
        quiet += 1
        if (quiet > 2 * CONTEXT) break
      } else {
        quiet = 0
        end = next
      }
    }
    const stop = Math.min(edits.length, end + 1 + CONTEXT)
    const lines = edits.slice(start, stop)
    out.push(blockLine(lines, before, after))
    for (const edit of lines)
      out.push(
        edit.kind +
          (edit.kind === '+' ? after[edit.after] : before[edit.before]),
      )
    index = stop
  }
  return out
}

/**
 * `@@` and the block the hunk's first change is in: the nearest line above that change with less
 * indentation, without its indentation, its list dash and its trailing colon (`@@ main
 * [ref=e293]`), so the agent can snapshot that block by its ref. Looked up in the new text, or
 * in the old one for a hunk that only removes lines. `@@` alone at the top level.
 */
function blockLine(
  lines: readonly Edit[],
  before: readonly string[],
  after: readonly string[],
): string {
  const first = lines.find((edit) => edit.kind !== ' ')
  if (first === undefined) return '@@'
  const removesOnly = lines.every((edit) => edit.kind !== '+')
  const text = removesOnly ? before : after
  const position = removesOnly ? first.before : first.after
  const changed = first.kind === '+' ? after[first.after] : before[first.before]
  const depth = indentOf(changed)
  for (let index = position - 1; index >= 0; index--) {
    if (indentOf(text[index]) < depth)
      return `@@ ${text[index]
        .trimStart()
        .replace(/^- /, '')
        .replace(/:$/, '')}`
  }
  return '@@'
}

function indentOf(line: string): number {
  return line.length - line.trimStart().length
}
