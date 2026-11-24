/**
 * A `covered` pointer action's error, as the script receives it (covered-click spec section 2;
 * Rust writes the rest in neo-playwright's action/cover.rs). Rust writes every line of the
 * message and, apart from it, the cover's own snapshot as the lines to print under "On top"
 * (`block`, printed by neo-snapshot's `cover_block` in a script's default snapshot form, four
 * spaces before each line, cut at 1,500 units). It arrives as the error's `cover` field.
 *
 * Here the block goes in right under the message's "On top" line, so its refs read as the agent
 * knows them. The script keeps the rest as `error.cover`: `{ ref, role, name, kind, controls }`.
 * A cover with no block leaves the message as Rust wrote it, whose controls and "Do" lines stand
 * without it. The block stays out of Rust's own message so that the operation's record, which
 * keeps that message, never carries it.
 */

/** The line Rust writes the cover on; the block goes right under it. */
const ON_TOP = '\n  On top: '

/** What a script reads as `error.cover`. */
type ScriptCover = {
  ref: string
  role: string
  name: string
  kind: string
  controls: { ref: string; role: string; name: string }[]
}

/** Places the cover's block into `error`'s message and leaves the script its `cover`. */
export function shapeCovered(error: Error): void {
  const cover: unknown = Reflect.get(error, 'cover')
  if (typeof cover !== 'object' || cover === null) return
  const block: unknown = Reflect.get(cover, 'block')
  if (typeof block === 'string' && block !== '')
    error.message = underOnTop(error.message, block)
  const { block: _left, ...kept } = cover as ScriptCover & {
    block?: unknown
  }
  Reflect.set(error, 'cover', kept)
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
