/**
 * Which engine prints a snapshot's text, and the compare switch that runs both while both exist.
 *
 *   'prelude'  the snapshot op answers the tree; the prelude renders it (snapshot.ts,
 *              snapshot-v1.ts, snapshot-delta.ts) and keeps the tab's delta baseline through
 *              page.snapshotBaseline and page.snapshotPending (snapshot-late.ts)
 *   'rust'     the op is asked for the text (`format`); Rust prints it (neo-snapshot) and reads
 *              and records the baseline itself, inside that one op, after its event wait
 *
 * `SNAPSHOT_TEXT` is the switch, one line, in the shape of `LATE_SNAPSHOT`: the switch piece
 * flips it to 'rust' and the way back flips it again. Either way the option checks and their
 * messages stay here, in today's order (frame.ts `textCall`). Tests that pin rendering through
 * the built prelude build their own asset with it at 'prelude' (tests/support/prelude-runtime.ts
 * `oldPathPrelude`), so the flip needs no other change.
 *
 * The compare switch (design 2.3, layer 5) is a test harness's, never a host's. The harness
 * hands the realm a counter object as `__neo_compare_snapshots` before the prelude starts
 * (neo-playwright-tests/node/claw-loader.mjs, neo-playwright's tests/support/js_runner.rs,
 * tests/support/prelude-runtime.ts). On the 'prelude' path every text call then asks its one
 * tree op for Rust's text too (`compare: true`): Rust answers the tree and its text from that one
 * read of the page, records nothing and takes no event wait inside, so this path runs exactly as
 * without the switch, and its text is held to Rust's here. A difference fails the call naming
 * the first differing line's number and the two lengths, never a line's text. Every comparison
 * is counted on the harness's object, which the harness prints, because a comparison that
 * silently did not run is not a pass; a call either engine answered with the no-time line is
 * counted apart and not compared, since the two clocks may disagree.
 */

import { internalError } from './errors'
import type { RustText, SnapshotForm } from './generated'
import { NO_TIME } from './snapshot-delta'

/** Which engine prints a snapshot's text (see above). */
export const SNAPSHOT_TEXT: SnapshotEngine = 'rust'

export type SnapshotEngine = 'prelude' | 'rust'

/** The harness's counters, which the prelude adds to. */
export type SnapshotComparisons = {
  /** Calls whose two texts (and pending baselines) were compared. */
  compared: number
  /** Calls not compared because either engine answered the no-time line. */
  noTime: number
  /** Compared calls that differed: each also failed its call. */
  differed: number
}

/**
 * The harness's counters, or null without the switch. Read once, as the prelude starts and before
 * the agent's function exists, so a script that sets the global changes nothing.
 */
const comparisons: SnapshotComparisons | null =
  ((): SnapshotComparisons | null => {
    const counters: unknown = Reflect.get(globalThis, '__neo_compare_snapshots')
    return typeof counters === 'object' && counters !== null
      ? (counters as SnapshotComparisons)
      : null
  })()

/** Whether a test harness switched the comparison on. */
export const compareEngines: boolean = comparisons !== null

/** What the prelude's own path printed for one call, and what it records as pending. */
export type PreludeText = {
  text: string
  /** The pending baseline it records: a whole page in Neo's format, or a delta that read one. */
  pending: { text: string; form: SnapshotForm } | null
}

/**
 * Holds Rust's answer to one compared call to the prelude's: the text the script receives, and
 * the text and form each would record as pending. Throws a `neo / internal` error on the first
 * difference; a no-time answer from either engine is counted and not compared.
 */
export function compareSnapshots(
  api: string,
  rust: RustText,
  prelude: PreludeText,
): void {
  if (comparisons === null) return
  if (rust.snapshot.startsWith(NO_TIME) || prelude.text.startsWith(NO_TIME)) {
    comparisons.noTime += 1
    return
  }
  comparisons.compared += 1
  const difference =
    textDifference('text', rust.snapshot, prelude.text) ??
    pendingDifference(rust.pending ?? null, prelude.pending)
  if (difference === null) return
  comparisons.differed += 1
  throw internalError(api, `the two snapshot engines differ: ${difference}`)
}

/** Where two texts first differ, by line number and both lengths; null when they are equal. */
function textDifference(
  what: string,
  rust: string,
  prelude: string,
): string | null {
  if (rust === prelude) return null
  const rustLines = rust.split('\n')
  const preludeLines = prelude.split('\n')
  let line = 0
  while (
    line < rustLines.length &&
    line < preludeLines.length &&
    rustLines[line] === preludeLines[line]
  )
    line += 1
  return `${what} at line ${line + 1} (Rust's is ${rust.length} UTF-16 units, the prelude's ${prelude.length})`
}

/** Where two pending records differ, naming the field, never a text; null when they agree. */
function pendingDifference(
  rust: { text: string; form: SnapshotForm } | null,
  prelude: { text: string; form: SnapshotForm } | null,
): string | null {
  if (rust === null || prelude === null)
    return rust === prelude
      ? null
      : `pending baseline recorded by ${rust === null ? 'the prelude' : 'Rust'} only`
  const text = textDifference('pending text', rust.text, prelude.text)
  if (text !== null) return text
  for (const field of ['text', 'urls', 'maxTokens'] as const)
    if (rust.form[field] !== prelude.form[field])
      return `pending form's ${field}`
  // The prelude writes `inert` only when it is true, and so does Rust.
  if ((rust.form.inert === true) !== (prelude.form.inert === true))
    return "pending form's inert"
  return null
}
