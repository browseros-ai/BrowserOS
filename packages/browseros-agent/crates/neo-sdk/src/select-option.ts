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
  if (isString(values[0]))
    return {
      options: (values as string[]).map((valueOrLabel) => ({ valueOrLabel })),
    }
  return { options: values as SelectOption[] }
}
