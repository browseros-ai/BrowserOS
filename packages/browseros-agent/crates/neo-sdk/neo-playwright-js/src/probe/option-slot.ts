/**
 * An `<option>` of a collapsed `<select>` as Rust's APC tree holds it: data on its select, with no
 * node of its own (neo-snapshot-apc tree/option_slot.rs). A request names one as
 * `select.options[index]` of a select Rust hands over, with `count`, the length of the select's
 * list in the capture (`SetMember::Option`).
 *
 * That length is the witness. Once the list grew or shrank, `index` may name another option, so
 * the member is CHANGED, and the stage answers `detached` as for a member that left: Rust captures
 * again. A same-length reorder still passes; only an option id from the APC contract would close
 * that. The witness carries no page text, which a private select's capture holds only as bullets.
 */
import { ProbeError } from './world'

/** A select whose option list no longer has the length the capture saw. */
export const CHANGED: unique symbol = Symbol('option list changed')

/** Where a request says an option sits in its select's list. */
export type OptionSlot = { readonly index: number; readonly count: number }

/**
 * `select.options[slot.index]` of a live `select`, or CHANGED when its list's length is not the
 * capture's. Rust hands over only a `<select>`, and only an index below its count, so anything
 * else is a probe bug.
 */
export function optionInSlot(
  select: Element,
  slot: OptionSlot,
): HTMLOptionElement | typeof CHANGED {
  if (!isSelect(select))
    throw new ProbeError(
      'internal',
      `an option member names a <${select.localName}> as its select`,
    )
  const { options } = select
  if (options.length !== slot.count) return CHANGED
  const option = options.item(slot.index)
  if (!option)
    throw new ProbeError(
      'internal',
      `an option member names option ${slot.index} of a list of ${slot.count}`,
    )
  return option
}

/** By its name: the offline suite's DOM (linkedom) has no `HTMLSelectElement` global. */
function isSelect(element: Element): element is HTMLSelectElement {
  return element.localName === 'select' && 'options' in element
}
