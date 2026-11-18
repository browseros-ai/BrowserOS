/**
 * Writes the snapshot text cases (model.ts) from today's renderer, as two files the Rust
 * renderer's tests read and never run JavaScript for:
 *
 *   playwright.json   Playwright's format on trees with no Neo facts, written from the vendored
 *                     printer itself (`renderAriaSnapshotAsYaml`), with today's renderer held to
 *                     it byte for byte. They outlive the TypeScript renderer: the vendored files
 *                     stay, so these are rewritten at every engine upgrade and their diff is what
 *                     upstream now prints differently.
 *   neo.json          every other call, printed by today's renderer in the prelude. Frozen when
 *                     that renderer is deleted; from then on a rule change is a reviewed diff.
 *
 * build.ts writes both with the other generated files and `build.ts --check` fails when either is
 * stale, so a change to today's renderer shows as a diff of the cases in review. The Rust
 * renderer's tests are to read the same files, so while both engines exist a change to one fails
 * the other's check until the two agree.
 */

import { setCoverRenderer, shapeCovered } from '../../src/prelude/cover'
import { renderFullSnapshot, renderSnapshot } from '../../src/prelude/snapshot'
import { renderDelta, renderUncompared } from '../../src/prelude/snapshot-delta'
import {
  DEFAULT_MAX_TOKENS,
  renderSnapshotV1,
  withDefaults,
} from '../../src/prelude/snapshot-v1'
import {
  type AriaNodeJSON,
  renderAriaSnapshotAsYaml,
} from '../../src/prelude/vendored'
import type { SnapshotBaseline, SnapshotResult } from '../../src/prelude/wire'
import { attributes } from './attributes'
import { boundaries } from './boundaries'
import { clip } from './clip'
import { closedRoots } from './closed-roots'
import { control } from './control'
import { coveredBlock } from './covered-block'
import { delta } from './delta'
import { depth } from './depth'
import { escaping } from './escaping'
import { fractions } from './fractions'
import { inert } from './inert'
import { locator } from './locator'
import type {
  Call,
  Case,
  CaseDef,
  Chain,
  Earlier,
  Gap,
  Snapshot,
} from './model'
import { onTop } from './on-top'
import { partialFrames } from './partial-frames'
import { playwright } from './playwright'
import { rules } from './rules'
import { selects } from './selects'
import { unmarked } from './unmarked'

/** Every gap's module, in the order its cases are written. */
const GAPS: Record<string, Gap> = {
  control,
  inert,
  'on-top': onTop,
  unmarked,
  'closed-roots': closedRoots,
  'partial-frames': partialFrames,
  selects,
  locator,
  clip,
  rules,
  boundaries,
  delta,
  fractions,
  playwright,
  escaping,
  attributes,
  'covered-block': coveredBlock,
  depth,
}

/** Where the files go, relative to packages/browseros-agent: beside the crate they judge. */
const FOLDER = 'crates/neo-sdk/neo-snapshot/tests/data/cases'

/**
 * Together under 1 MB, as the Rust snapshot renderer design sets (section 2.4): the largest JSON the SDK tracks is
 * 1.14 MB, and these are read by a test on every gate run.
 */
const MAX_BYTES = 1_000_000

const ABOUT = [
  'Snapshot text cases: what today’s renderer prints for each call, byte for byte. Written by crates/neo-sdk/neo-playwright-js/scripts/snapshot-cases/write.ts through scripts/build.ts; do not edit by hand. The shape is documented in scripts/snapshot-cases/model.ts.',
  'A case renders `snapshots[case.snapshot]` with `case.call` and must print `case.text`; a whole page in Neo’s format also keeps `case.pending` as the tab’s pending baseline. A snapshot with `chain` instead of `json` is built in code: `depth` nodes, each a copy of `link` holding the next, the last `leaf`.',
  'Lone halves of a character are written as U+FFFD: the covered block’s cut is JavaScript’s own, and U+FFFD is what every reader outside the script engine receives (the Rust snapshot renderer design, 1.9). No other text holds one.',
]

/** The tree as Rust will hand it to the renderer: a fresh copy, so no render sees another's. */
function result(snapshot: Snapshot): SnapshotResult {
  const { json, chain, ...facts } = snapshot
  return {
    ...structuredClone(facts),
    json: chain ? built(chain) : structuredClone(json ?? []),
  }
}

function built(shape: Chain): AriaNodeJSON[] {
  let node: AriaNodeJSON = structuredClone(shape.leaf)
  for (let level = 1; level < shape.depth; level++)
    node = { ...structuredClone(shape.link), children: [node] }
  return [node]
}

/**
 * No fact beside the tree. Every fact is Neo's, so a Playwright-format call on a tree with one
 * tests a Neo rule, even when the rule is to leave the fact out: it belongs in neo.json, frozen
 * with today's renderer, not in playwright.json, which only the vendored printer writes.
 */
function factless(snapshot: Snapshot): boolean {
  const { json: _json, chain: _chain, partial, ...rest } = snapshot
  return partial.length === 0 && Object.keys(rest).length === 0
}

function earlier(
  given: Earlier | { keptBy: { snapshot: string; passed: object } },
  snapshots: Record<string, Snapshot>,
): Earlier {
  if (typeof given === 'string' || !('keptBy' in given)) return given
  const from = snapshots[given.keptBy.snapshot]
  if (!from)
    throw new Error(`keptBy names no snapshot ${given.keptBy.snapshot}`)
  const kept = renderSnapshotV1(result(from), withDefaults(given.keptBy.passed))
  return { text: kept.text, form: kept.form }
}

/** The message Rust writes for a covered click, around which the block is placed. */
const COVER_HEAD =
  'locator.click: the element is covered\n  On top: dialog "Cookies" [ref=e9]'
const COVER_TAIL = '\n  Its controls: button "Accept" [ref=e10]'

/**
 * The block the prelude puts under a covered click's "On top" line, or null for none: read back
 * from today's own placement (cover.ts `shapeCovered`) on a message of known shape.
 */
function coverBlock(json: AriaNodeJSON[] | null): string | null {
  // The prelude's entry (src/prelude/index.ts) gives cover.ts this same renderer.
  setCoverRenderer(
    (tree) =>
      renderSnapshotV1({ json: tree, partial: [] }, withDefaults({})).text,
  )
  const error = new Error(COVER_HEAD + COVER_TAIL)
  Reflect.set(error, 'cover', {
    ref: 'e9',
    role: 'dialog',
    name: 'Cookies',
    kind: 'dialog',
    controls: [],
    snapshot: json,
  })
  shapeCovered(error)
  if (error.message === COVER_HEAD + COVER_TAIL) return null
  return error.message.slice(COVER_HEAD.length + 1, -COVER_TAIL.length)
}

/** Today's answer to one call: the text and, for a whole page's default format, what it keeps. */
function render(
  call: Call,
  snapshot: Snapshot | null,
): { text: string | null; pending?: SnapshotBaseline } {
  if ('cover' in call) {
    if (snapshot !== null && !factless(snapshot))
      throw new Error(
        'a cover renders its tree alone: give it a snapshot with no facts',
      )
    return {
      text: coverBlock(snapshot === null ? null : result(snapshot).json),
    }
  }
  if (snapshot === null) throw new Error('only a cover may have no snapshot')
  const tree = result(snapshot)
  if ('playwright' in call) {
    const text = renderSnapshot(tree)
    if (!factless(snapshot)) return { text }
    // Neo walks the tree with its own stack and asks the vendored printer one node at a time;
    // on a tree with no facts that must be the vendored printer's own text, byte for byte.
    const vendored = renderAriaSnapshotAsYaml(tree.json)
    if (text !== vendored)
      throw new Error("today's Playwright text is not the vendored printer's")
    return { text: vendored }
  }
  if ('full' in call)
    return {
      text: renderFullSnapshot(tree, call.full.maxTokens ?? DEFAULT_MAX_TOKENS),
    }
  if ('element' in call)
    return { text: renderSnapshotV1(tree, withDefaults(call.element)).text }
  if ('page' in call) {
    const { text, form } = renderSnapshotV1(tree, withDefaults(call.page))
    return { text, pending: { text, form } }
  }
  const { passed, earlier: found } = call.delta
  if (found === 'notRead') return { text: renderUncompared(tree, passed) }
  const answer = renderDelta(found === 'nothing' ? null : found, tree, passed)
  return {
    text: answer.returned,
    pending: { text: answer.baseline.text, form: answer.baseline.form },
  }
}

/** A text as the file holds it; only a covered block may hold half a character. */
function whole(text: string | null, call: Call, name: string): string | null {
  if (text === null || text.isWellFormed()) return text
  if (!('cover' in call))
    throw new Error(
      `${name}: a text other than a cover's holds half a character`,
    )
  return text.toWellFormed()
}

/**
 * One case as today's renderer answers it. Exported, with `caseDocument`, so the saved real
 * pages kept outside the repository are written in this same shape by this same code.
 */
export function writeCase(
  def: CaseDef,
  snapshots: Record<string, Snapshot>,
): Case {
  const snapshot = def.snapshot === null ? null : snapshots[def.snapshot]
  if (snapshot === undefined)
    throw new Error(`${def.name} names no snapshot ${def.snapshot}`)
  const call: Call =
    'delta' in def.call
      ? {
          delta: {
            passed: def.call.delta.passed,
            earlier: earlier(def.call.delta.earlier, snapshots),
          },
        }
      : def.call
  let answer: ReturnType<typeof render>
  try {
    answer = render(call, snapshot)
  } catch (error) {
    throw new Error(
      `${def.name}: ${error instanceof Error ? error.message : error}`,
    )
  }
  const { text, pending } = answer
  return {
    name: def.name,
    snapshot: def.snapshot,
    call,
    text: whole(text, call, def.name),
    ...(pending
      ? {
          pending: {
            text: whole(pending.text, call, def.name) ?? '',
            form: pending.form,
          },
        }
      : {}),
  }
}

/** One file: its own snapshots, so each file reads alone, then one case per line. */
export function caseDocument(
  cases: Case[],
  snapshots: Record<string, Snapshot>,
): string {
  const used = [
    ...new Set(
      cases.flatMap((each) => (each.snapshot === null ? [] : [each.snapshot])),
    ),
  ]
  const lines = [
    '{',
    ` "about": ${JSON.stringify(ABOUT)},`,
    ' "snapshots": {',
    used
      .map(
        (name) =>
          `  ${JSON.stringify(name)}: ${JSON.stringify(snapshots[name])}`,
      )
      .join(',\n'),
    ' },',
    ' "cases": [',
    cases.map((each) => `  ${JSON.stringify(each)}`).join(',\n'),
    ' ]',
    '}',
  ]
  return `${lines.join('\n')}\n`
}

/** Both files, for `gaps` (every gap by default). Throws on a case the files could not hold. */
export function snapshotCaseOutputs(
  gaps: Record<string, Gap> = GAPS,
): { path: string; text: string }[] {
  const snapshots: Record<string, Snapshot> = {}
  const defs: CaseDef[] = []
  for (const [key, gap] of Object.entries(gaps)) {
    // A name says which gap it closes, so a reader of one case knows the rule it pins.
    const named = (name: string) => {
      if (!name.startsWith(`${key}/`))
        throw new Error(`${name} is in the ${key} gap: name it ${key}/...`)
    }
    for (const [name, snapshot] of Object.entries(gap.snapshots)) {
      named(name)
      if (Object.hasOwn(snapshots, name))
        throw new Error(`two snapshots are named ${name}`)
      snapshots[name] = snapshot
    }
    for (const def of gap.cases) named(def.name)
    defs.push(...gap.cases)
  }
  const names = new Set<string>()
  const vendored: Case[] = []
  const neo: Case[] = []
  for (const def of defs) {
    if (names.has(def.name)) throw new Error(`two cases are named ${def.name}`)
    names.add(def.name)
    const each = writeCase(def, snapshots)
    const snapshot = def.snapshot === null ? null : snapshots[def.snapshot]
    if ('playwright' in each.call && snapshot && factless(snapshot))
      vendored.push(each)
    else neo.push(each)
  }
  const outputs = [
    {
      path: `${FOLDER}/playwright.json`,
      text: caseDocument(vendored, snapshots),
    },
    { path: `${FOLDER}/neo.json`, text: caseDocument(neo, snapshots) },
  ]
  const bytes = outputs.reduce(
    (sum, output) => sum + Buffer.byteLength(output.text),
    0,
  )
  if (bytes > MAX_BYTES)
    throw new Error(
      `the snapshot cases are ${bytes} bytes, over the ${MAX_BYTES} they may take together`,
    )
  return outputs
}
