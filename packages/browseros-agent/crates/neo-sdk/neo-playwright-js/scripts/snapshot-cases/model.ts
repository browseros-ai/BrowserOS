/**
 * The shape of the snapshot text cases: what today's TypeScript renderer prints for every rule,
 * written down so the Rust renderer that replaces it (crate `neo-snapshot`) is judged against
 * these bytes, call by call. One case is one call of the renderer as the snapshot operation will
 * make it:
 *
 *   snapshot  the stitched tree and the facts Rust reads beside it (partial frames, the on-top
 *             layers, unread closed roots, an unfinished check for clickable elements)
 *   call      which text: Playwright's format, `mode: 'full'`, Neo's default format for a
 *             locator or a whole page, a delta, or the covered click's block
 *   text      the string the script receives (the block for a cover; null when there is none)
 *   pending   for a whole page in Neo's format: the text and form kept as the tab's pending
 *             baseline, which a later `{ delta: true }` compares against
 *
 * Every tree is built here by hand, so no page's text is ever committed. A tree deeper than a
 * JSON file may nest (serde_json refuses past 128 levels) is a `chain`, built in code on both
 * sides. The files are written by write.ts.
 */
import type { PassedV1Options } from '../../src/prelude/snapshot-v1'
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import type { SnapshotBaseline, SnapshotResult } from '../../src/prelude/wire'

/** The options a script passed to a default-format call, each absent when left out. */
export type Passed = PassedV1Options

/** What Rust reads beside the tree, each a fact about the page and none a rendering choice. */
export type Facts = Pick<
  SnapshotResult,
  'partial' | 'onTop' | 'closedRoots' | 'unmarked'
>

/**
 * A tree too deep to be a JSON file: `depth` nodes, each of the first `depth - 1` a copy of
 * `link` whose only child is the next, the last one `leaf`. So `json` is `[link{children:
 * [link{… [leaf]}]}]`, and `depth: 1` is `[leaf]`. `link` has no children of its own.
 */
export type Chain = {
  depth: number
  link: Omit<AriaNodeJSON, 'children'>
  leaf: AriaNodeJSON
}

/**
 * What Rust hands the renderer, as a case file holds it: kept once per file under `snapshots`,
 * named, and shared by every case that renders it. Rust's test decodes it into the crate's tree.
 */
export type Snapshot = Facts &
  ({ json: AriaNodeJSON[]; chain?: never } | { chain: Chain; json?: never })

/**
 * What a delta call found for the tab: the stored text was not read because the call's time ran
 * out (`notRead`), there was none (`nothing`), or the stored text and how it was rendered.
 */
export type Earlier = 'notRead' | 'nothing' | SnapshotBaseline

/** One call of the renderer, one variant per kind of text (the Rust snapshot renderer design's `Call`). */
export type Call =
  | { playwright: Record<string, never> }
  /** `maxTokens` left out is the default cap, as the operation will apply it. */
  | { full: { maxTokens?: number } }
  /** A locator's snapshot in Neo's format: never a baseline. */
  | { element: Passed }
  /** A whole page in Neo's format without `delta`: its text becomes the pending baseline. */
  | { page: Passed }
  | { delta: { passed: Passed; earlier: Earlier } }
  /** The block a covered click's error carries under its "On top" line; the tree alone. */
  | { cover: Record<string, never> }

/** One written case: a call on a named snapshot and what today's renderer answers. */
export type Case = {
  name: string
  /** The snapshot's name in the same file; null only for a cover whose tree is null. */
  snapshot: string | null
  call: Call
  text: string | null
  pending?: SnapshotBaseline
}

/**
 * A stored baseline as a real one arises: what a whole-page call with `passed` kept on the
 * snapshot named `snapshot`. The writer resolves it into that call's text and form.
 */
export type KeptBy = { keptBy: { snapshot: string; passed: Passed } }

/** A call as a case module states it: a delta's earlier text may name the call that kept it. */
export type CallDef =
  | Exclude<Call, { delta: unknown }>
  | { delta: { passed: Passed; earlier: Earlier | KeptBy } }

export type CaseDef = { name: string; snapshot: string | null; call: CallDef }

/** One gap of the Rust snapshot renderer design's table (section 2.4): its trees, and the calls made on them. */
export type Gap = { snapshots: Record<string, Snapshot>; cases: CaseDef[] }

export function tree(
  json: AriaNodeJSON[],
  facts: Partial<Facts> = {},
): Snapshot {
  return { json, partial: [], ...facts }
}

export function chain(shape: Chain, facts: Partial<Facts> = {}): Snapshot {
  return { chain: shape, partial: [], ...facts }
}

export const playwright = (): CallDef => ({ playwright: {} })
export const full = (maxTokens?: number): CallDef => ({
  full: maxTokens === undefined ? {} : { maxTokens },
})
export const element = (passed: Passed = {}): CallDef => ({ element: passed })
export const page = (passed: Passed = {}): CallDef => ({ page: passed })
export const cover = (): CallDef => ({ cover: {} })
export const delta = (passed: Passed, earlier: Earlier | KeptBy): CallDef => ({
  delta: { passed, earlier },
})
/** The baseline a whole-page call with `passed` on `snapshot` kept. */
export const keptBy = (snapshot: string, passed: Passed = {}): KeptBy => ({
  keptBy: { snapshot, passed },
})

/**
 * One case per entry of `calls` on `snapshot`, each named `<snapshot>/<key>`, so a case's name
 * says which tree and which call.
 */
export function calls(
  snapshot: string,
  each: Record<string, CallDef>,
): CaseDef[] {
  return Object.entries(each).map(([key, call]) => ({
    name: `${snapshot}/${key}`,
    snapshot,
    call,
  }))
}

/** The calls most rules are worth seeing through: every format on one tree. */
export function everyFormat(): Record<string, CallDef> {
  return {
    playwright: playwright(),
    full: full(),
    element: element(),
    page: page(),
    'page-text': page({ text: true }),
    'page-notext': page({ text: false }),
    'delta-first': delta({}, 'nothing'),
    'delta-uncompared': delta({}, 'notRead'),
  }
}
