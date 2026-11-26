/**
 * An `<option>` of a collapsed `<select>` as Rust's APC tree holds it: data on its select, with no
 * node of its own (neo-snapshot-apc tree/option_slot.rs). Both directions of that address live
 * here:
 *
 *   forward  a request names one as `select.options[index]` of a select Rust hands over, with
 *            `count`, the length of the select's list in the capture (`SetMember::Option`,
 *            optionInSlot)
 *   reverse  `pinnedAt` answers a pinned `<option>` with its select, index and the list's length
 *            now (slotOf), and Rust's tree finds the option's slot by them
 *            (`SnapshotTree::node_at`)
 *
 * That length is the witness. Once the list grew or shrank, `index` may name another option, so
 * the member is CHANGED, and the stage answers `detached` as for a member that left: Rust captures
 * again; the reverse join likewise finds no slot. A same-length reorder still passes; only an
 * option id from the APC contract would close that. The witness carries no page text, which a
 * private select's capture holds only as bullets.
 */
import type { PinnedAtRequest } from './protocol'
import { ProbeError, type World } from './world'

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

/**
 * `pinnedAt`'s reply, which Rust reads by deep serialization (probe_call.rs
 * `pinned_place_in_document`): the element alone, or an `<option>` with the select that lists it,
 * its index in `select.options` and that list's length.
 */
export type PinnedPlace =
  | readonly [element: Element]
  | readonly [
      element: Element,
      select: HTMLSelectElement,
      index: number,
      count: number,
    ]

/**
 * Where the pinned element sits, for Rust's reverse join (snapshot::element_in_tree): null once
 * the pin is gone or its element is not connected in this world's document (design 6.2: one the
 * page moved into another document is not the one the capture saw).
 */
export function pinnedAt(
  world: World,
  request: PinnedAtRequest,
): PinnedPlace | null {
  const element = world.pins.held(request.pin)
  if (
    !element?.isConnected ||
    element.ownerDocument !== world.injected.document
  )
    return null
  const slot = slotOf(element)
  return slot ? [element, slot.select, slot.index, slot.count] : [element]
}

/**
 * `element`'s place in the option list of the select it sits in, the list Chromium copies into
 * APC's `select_options` (`GetOptionList()`, which is `select.options`): undefined when no select
 * lists it, as for an option of a `<datalist>`, outside any select, or any other element. The
 * index comes from the list itself, so an option inside an `<optgroup>` counts where the list puts
 * it; `option.index` is not read, since the offline suite's DOM (linkedom) has none.
 */
function slotOf(
  element: Element,
): { select: HTMLSelectElement; index: number; count: number } | undefined {
  const select = element.closest('select')
  if (!select || !isSelect(select)) return undefined
  const { options } = select
  const index = Array.prototype.indexOf.call(options, element)
  return index < 0 ? undefined : { select, index, count: options.length }
}

/** By its name: the offline suite's DOM (linkedom) has no `HTMLSelectElement` global. */
function isSelect(element: Element): element is HTMLSelectElement {
  return element.localName === 'select' && 'options' in element
}
