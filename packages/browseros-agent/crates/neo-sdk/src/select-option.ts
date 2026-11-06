/** Playwright's `client/elementHandle.ts` value conversion, without element handles.
 * Neo has no ElementHandle objects; those wire references are refused by the server. */
import type { FrameSelectOptionParams } from '@browseros/neo-runtime-spec/serve'
import { isString } from '../vendor/playwright/isomorphic/stringUtils'

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
