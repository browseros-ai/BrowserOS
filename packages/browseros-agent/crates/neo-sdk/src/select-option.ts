/** Playwright's `client/elementHandle.ts` value conversion, without element handles: Playwright
 * sends ElementHandle values as `elements`, and its injected script matches options by node
 * identity, which Neo cannot pass to its injected script yet. Such a value is refused here,
 * before anything is sent; Rust refuses `elements` on the wire too (`dispatch.rs`). */
import type { FrameSelectOptionParams } from '@browseros/neo-runtime-spec/serve'
import { isString } from '../vendor/playwright/isomorphic/stringUtils'
import { JSHandle } from './js-handle'

/** Deviation: `client/elementHandle.ts` `convertSelectOptionValues`: `return { elements: (values as ElementHandle[]).map((v: ElementHandle) => v._elementChannel) };` Playwright selects the very option nodes ElementHandle values name; Neo refuses an ElementHandle (or any JSHandle) value before anything is sent, where it would otherwise travel as an option object matching the first option. */
export const ELEMENT_VALUES_UNSUPPORTED =
  'Neo does not support ElementHandle values in selectOption yet'

export type SelectOption = NonNullable<
  FrameSelectOptionParams['options']
>[number]
export type SelectOptionValues =
  | string
  | SelectOption
  | string[]
  | SelectOption[]
  | null

/**
 * Playwright's `convertSelectOptionValues` and its protocol validation, over one copy of the
 * caller's value: it is read once, element by element and field by field, into arrays and
 * objects the SDK owns, so none of the caller's own methods runs (an array proxy's `some` or
 * `map`) and what is checked is exactly what is sent.
 */
export function convertSelectOptionValues(
  values: SelectOptionValues,
): Pick<FrameSelectOptionParams, 'options'> {
  if (values === null) return {}
  const list = snapshot(values)
  if (!list.length) return {}
  for (let i = 0; i < list.length; i++) {
    if (list[i] === null)
      throw new Error(`options[${i}]: expected object, got null`)
  }
  for (let i = 0; i < list.length; i++) {
    if (list[i] instanceof JSHandle) throw new Error(ELEMENT_VALUES_UNSUPPORTED)
  }
  const strings = isString(list[0])
  const options: SelectOption[] = []
  for (let i = 0; i < list.length; i++) {
    options[i] = copyOption(strings ? { valueOrLabel: list[i] } : list[i], i)
  }
  return { options }
}

/** Neo: the caller's value as an array the SDK owns, each element read once by index. */
function snapshot(values: unknown): unknown[] {
  if (!Array.isArray(values)) return [values]
  const length: unknown = values.length
  if (typeof length !== 'number' || !Number.isSafeInteger(length) || length < 0)
    throw new Error('options: expected array')
  const list: unknown[] = []
  for (let i = 0; i < length; i++) list[i] = values[i]
  return list
}

/** Neo: one option, its four protocol fields read once and validated, as a new object. */
function copyOption(option: unknown, index: number): SelectOption {
  if (typeof option !== 'object' || option === null)
    throw new Error(`options[${index}]: expected object`)
  const source = option as Record<string, unknown>
  const valueOrLabel = source.valueOrLabel
  const value = source.value
  const label = source.label
  const position = source.index
  const copy: SelectOption = {}
  if (valueOrLabel !== undefined)
    copy.valueOrLabel = text(valueOrLabel, index, 'valueOrLabel')
  if (value !== undefined) copy.value = text(value, index, 'value')
  if (label !== undefined) copy.label = text(label, index, 'label')
  if (position !== undefined) {
    if (typeof position !== 'number' || !Number.isInteger(position))
      throw new Error(`options[${index}].index: expected integer`)
    copy.index = position
  }
  return copy
}

function text(value: unknown, index: number, field: string): string {
  if (typeof value !== 'string')
    throw new Error(`options[${index}].${field}: expected string`)
  return value
}
