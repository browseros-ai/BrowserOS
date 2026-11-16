/**
 * A `covered` pointer action's error, as the script receives it (covered-click spec section 2;
 * Rust writes the rest in neo-playwright's action/cover.rs). Rust writes every line of the
 * message but one: the cover's own snapshot, which only this runtime renders, as it renders
 * every snapshot. It arrives as the error's `cover` field, with its JSON.
 *
 * Here the snapshot goes in right under the message's "On top" line, in the form a script's
 * own default `ariaSnapshot()` has, so its refs read as the agent knows them, cut at 1,500
 * characters. The script keeps the rest as `error.cover`: `{ ref, role, name, kind, controls }`.
 * A snapshot that is absent, empty or fails to render leaves the message as Rust wrote it, whose
 * controls and "Do" lines stand without it.
 */

import type { AriaSnapshotJSON } from './vendored'

/** The most of the cover's rendered snapshot the message carries. */
const SNAPSHOT_CHARS = 1500

/**
 * Renders a snapshot tree in a script's default snapshot form. The prelude's entry (index.ts) sets
 * it once. errors.ts, part of every call's error path, imports this module, and a static import
 * of the snapshot renderer here would make the renderer and its vendored Playwright modules
 * dependencies of that core path.
 */
let renderSnapshot: ((json: AriaSnapshotJSON) => string) | undefined

/** Gives this module the snapshot renderer; the prelude's entry calls it once. */
export function setCoverRenderer(
  render: (json: AriaSnapshotJSON) => string,
): void {
  renderSnapshot = render
}

/** The line Rust writes the cover on; the snapshot goes right under it. */
const ON_TOP = '\n  On top: '

/** What a script reads as `error.cover`. */
type ScriptCover = {
  ref: string
  role: string
  name: string
  kind: string
  controls: { ref: string; role: string; name: string }[]
}

/** Renders the cover's snapshot into `error`'s message and leaves the script its `cover`. */
export function shapeCovered(error: Error): void {
  const cover: unknown = Reflect.get(error, 'cover')
  if (typeof cover !== 'object' || cover === null) return
  const snapshot: unknown = Reflect.get(cover, 'snapshot')
  const lines = snapshotLines(snapshot)
  if (lines !== '') error.message = underOnTop(error.message, lines)
  const { snapshot: _left, ...kept } = cover as ScriptCover & {
    snapshot?: unknown
  }
  Reflect.set(error, 'cover', kept)
}

/** The snapshot as the message prints it, each line under "On top", or '' for none. */
function snapshotLines(snapshot: unknown): string {
  if (typeof snapshot !== 'object' || snapshot === null || !renderSnapshot)
    return ''
  let text: string
  try {
    text = renderSnapshot(snapshot as AriaSnapshotJSON)
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

/** `message` with `lines` right after its "On top" line; unchanged when it has none. */
function underOnTop(message: string, lines: string): string {
  const at = message.indexOf(ON_TOP)
  if (at < 0) return message
  const end = message.indexOf('\n', at + ON_TOP.length)
  return end < 0
    ? `${message}\n${lines}`
    : `${message.slice(0, end)}\n${lines}${message.slice(end)}`
}
