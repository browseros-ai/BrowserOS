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

export function convertSelectOptionValues(
  values: SelectOptionValues,
): Pick<FrameSelectOptionParams, 'options'> {
  if (values === null) return {}
  if (!Array.isArray(values)) values = [values] as string[] | SelectOption[]
  if (!values.length) return {}
  for (let i = 0; i < values.length; i++) {
    if (values[i] === null)
      throw new Error(`options[${i}]: expected object, got null`)
  }
  if (values.some((value: unknown) => value instanceof JSHandle))
    throw new Error(ELEMENT_VALUES_UNSUPPORTED)
  const options: SelectOption[] = isString(values[0])
    ? (values as string[]).map((valueOrLabel) => ({ valueOrLabel }))
    : (values as SelectOption[])
  for (const [index, option] of options.entries()) {
    if (typeof option !== 'object' || option === null)
      throw new Error(`options[${index}]: expected object`)
    for (const field of ['valueOrLabel', 'value', 'label'] as const) {
      if (option[field] !== undefined && typeof option[field] !== 'string')
        throw new Error(`options[${index}].${field}: expected string`)
    }
    if (option.index !== undefined && !Number.isInteger(option.index))
      throw new Error(`options[${index}].index: expected integer`)
  }
  return { options }
}
