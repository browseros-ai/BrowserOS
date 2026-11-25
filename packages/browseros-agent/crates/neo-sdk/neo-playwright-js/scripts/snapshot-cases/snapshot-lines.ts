/**
 * The covered click's block as the prelude rendered and cut it before Rust did: the oracle the
 * Rust port (neo-snapshot's `cover_block`) is judged by, on the committed cover cases and on
 * generated trees, until the TypeScript renderer is removed. Moved unchanged from the prelude's
 * `cover.ts`, except that it calls the renderer the prelude's entry used to hand that module
 * (`setCoverRenderer`) directly. It is not part of the prelude's bundle: only answer.ts, which
 * build.ts bundles for the tests, imports it.
 */

import { renderSnapshotV1, withDefaults } from '../../src/prelude/snapshot-v1'
import type { AriaSnapshotJSON } from '../../src/prelude/vendored'

/** The most of the cover's rendered snapshot the message carries. */
const SNAPSHOT_CHARS = 1500

/** The snapshot as the message prints it, each line under "On top", or '' for none. */
export function snapshotLines(snapshot: unknown): string {
  if (typeof snapshot !== 'object' || snapshot === null) return ''
  let text: string
  try {
    text = renderSnapshotV1(
      { json: snapshot as AriaSnapshotJSON, partial: [] },
      withDefaults({}),
    ).text
  } catch {
    return ''
  }
  const kept: string[] = []
  let used = 0
  for (const line of text.split('\n')) {
    if (line.trim() === '') continue
    if (used + line.length + 1 > SNAPSHOT_CHARS) {
      // A long text prints as one line: keep its start up to the limit, not nothing.
      const room = SNAPSHOT_CHARS - used - 1
      if (room > 0) kept.push(`    ${line.slice(0, room)}…`)
      kept.push('    … (the rest of its snapshot is cut)')
      break
    }
    used += line.length + 1
    kept.push(`    ${line}`)
  }
  return kept.join('\n')
}
