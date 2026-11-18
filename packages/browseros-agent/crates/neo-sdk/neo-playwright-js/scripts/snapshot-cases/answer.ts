/**
 * Today's answer to one renderer call: which function of today's TypeScript renderer a `Call`
 * means, and what it prints. Two judges of the Rust renderer (crate `neo-snapshot`) share it,
 * so a committed case and a generated tree are answered by the same dispatch:
 *
 *   write.ts   writes the committed cases with it
 *   build.ts   bundles it for QuickJS as neo-snapshot's test bundle of today's renderer, which
 *              the crate's differential calls on seeded random trees beside the Rust
 */

import { setCoverRenderer, shapeCovered } from '../../src/prelude/cover'
import { renderFullSnapshot, renderSnapshot } from '../../src/prelude/snapshot'
import { renderDelta, renderUncompared } from '../../src/prelude/snapshot-delta'
import {
  DEFAULT_MAX_TOKENS,
  renderSnapshotV1,
  withDefaults,
} from '../../src/prelude/snapshot-v1'
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import type { SnapshotBaseline, SnapshotResult } from '../../src/prelude/wire'
import type { Call } from './model'

/** The text a script receives (the block for a cover, null for none) and, for a whole page in Neo's format, what it keeps. */
export type Answer = { text: string | null; pending?: SnapshotBaseline }

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

/**
 * Today's answer to `call` on `result`, the tree and facts as Rust hands them over. Only a
 * cover may have no result: its tree is null. `result` is the renderer's to change, so pass a
 * fresh copy.
 */
export function todaysAnswer(
  call: Call,
  result: SnapshotResult | null,
): Answer {
  if ('cover' in call) return { text: coverBlock(result?.json ?? null) }
  if (result === null) throw new Error('only a cover may have no snapshot')
  if ('playwright' in call) return { text: renderSnapshot(result) }
  if ('full' in call)
    return {
      text: renderFullSnapshot(
        result,
        call.full.maxTokens ?? DEFAULT_MAX_TOKENS,
      ),
    }
  if ('element' in call)
    return { text: renderSnapshotV1(result, withDefaults(call.element)).text }
  if ('page' in call) {
    const { text, form } = renderSnapshotV1(result, withDefaults(call.page))
    return { text, pending: { text, form } }
  }
  const { passed, earlier: found } = call.delta
  if (found === 'notRead') return { text: renderUncompared(result, passed) }
  const answer = renderDelta(found === 'nothing' ? null : found, result, passed)
  return {
    text: answer.returned,
    pending: { text: answer.baseline.text, form: answer.baseline.form },
  }
}
