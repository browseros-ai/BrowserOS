/** Playwright's `client/elementHandle.ts` value conversion: ElementHandle values travel as
 * `elements`, which neo-runtime adopts into the select's world and Playwright's injected script
 * matches by node identity (`method_router.rs`, `forms.rs`); any other value as option objects. */
import type { FrameSelectOptionParams } from '@browseros/neo-runtime-spec/serve'
import { isString } from '../vendor/playwright/isomorphic/stringUtils'
import { ElementHandle } from './element-handle'

export type SelectOption = NonNullable<
  FrameSelectOptionParams['options']
>[number]
/** Playwright's `selectOption` values: strings, option objects or ElementHandles, alone or in
 * an array. */
export type SelectOptionValues =
  | string
  | ElementHandle
  | SelectOption
  | string[]
  | ElementHandle[]
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
): Pick<FrameSelectOptionParams, 'options' | 'elements'> {
  if (values === null) return {}
  const list = snapshot(values)
  if (!list.length) return {}
  for (let i = 0; i < list.length; i++) {
    if (list[i] === null)
      throw new Error(`options[${i}]: expected object, got null`)
  }
  if (list[0] instanceof ElementHandle) return { elements: elementsOf(list) }
  const strings = isString(list[0])
  const options: SelectOption[] = []
  for (let i = 0; i < list.length; i++) {
    options[i] = copyOption(strings ? { valueOrLabel: list[i] } : list[i], i)
  }
  return { options }
}

/** Playwright: `client/elementHandle.ts` `convertSelectOptionValues`: `return { elements: (values as ElementHandle[]).map((v: ElementHandle) => v._elementChannel) };`
 *
 * Each value's channel, read once: a value that is no ElementHandle has none, and Playwright's
 * client fails with a TypeError before sending. A disposed handle still names its guid, which
 * neo-runtime then refuses, as Playwright's dispatcher does. */
function elementsOf(list: unknown[]): FrameSelectOptionParams['elements'] {
  const elements: { guid: string }[] = []
  for (let i = 0; i < list.length; i++) {
    const value = list[i]
    // Playwright's client reads `v._elementChannel` of each value and its serializer then reads
    // that channel's `_object`, which throws a TypeError for a value that has none.
    if (!(value instanceof ElementHandle))
      throw new TypeError(
        `elements[${i}]: expected channel ElementHandle, got ${typeof value}`,
      )
    elements[i] = { guid: value._guid }
  }
  // The generated `ElementHandleChannel` has no values in neo-runtime-spec; the wire is the guid.
  return elements as unknown as FrameSelectOptionParams['elements']
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
