/**
 * Turning a script's option bags and arguments into wire fields.
 *
 * playwright-core's protocol validator drops option keys it does not know, so a script may pass
 * extra ones. Rust decodes with `deny_unknown_fields`, so each op sends only the keys its wire
 * type names (`pick`). Evaluate arguments travel as JSON in v1; a Playwright handle passed as an
 * argument is refused here with what to call instead, before the bootstrap's codec would refuse
 * it as an unbranded object.
 */
import type { SelectOption } from '../probe/protocol'
import { clientError } from './errors'
import type { ElementBrand } from './wire'

/** The defined values of `keys` in `source`. */
export function pick<T extends object, K extends keyof T>(
  source: T | undefined,
  keys: readonly K[],
): Pick<T, K> {
  const picked = {} as Pick<T, K>
  if (!source) return picked
  for (const key of keys)
    if (source[key] !== undefined) picked[key] = source[key]
  return picked
}

export const POINTER_KEYS = [
  'button',
  'clickCount',
  'delay',
  'force',
  'modifiers',
  'noWaitAfter',
  'position',
  'scroll',
  'steps',
  'trial',
] as const
export const DBLCLICK_KEYS = POINTER_KEYS.filter((key) => key !== 'clickCount')
export const HOVER_KEYS = [
  'force',
  'modifiers',
  'noWaitAfter',
  'position',
  'scroll',
  'trial',
] as const
export const CHECK_KEYS = [
  'force',
  'noWaitAfter',
  'position',
  'scroll',
  'trial',
] as const
export const DRAG_KEYS = [
  'force',
  'noWaitAfter',
  'scroll',
  'sourcePosition',
  'targetPosition',
  'steps',
  'trial',
] as const
export const FILL_KEYS = ['force', 'noWaitAfter'] as const
export const KEY_KEYS = ['delay', 'noWaitAfter'] as const
/** The screenshot options Rust captures with; frame.ts refuses the rest. */
export const SCREENSHOT_KEYS = [
  'type',
  'quality',
  'omitBackground',
  'fullPage',
  'clip',
  'animations',
  'caret',
  'style',
  'scale',
] as const

/** Objects the prelude hands to scripts (pages, frames, locators, handles), by identity. */
const handles = new WeakSet<object>()

export function markHandle(object: object): void {
  handles.add(object)
}

/** One of the objects the prelude hands to scripts. */
export function isHandle(value: unknown): boolean {
  return typeof value === 'object' && value !== null && handles.has(value)
}

type SelectOptionValue = {
  value?: string
  label?: string
  index?: number
  valueOrLabel?: string
}

/**
 * upstream's convertSelectOptionValues, onto the probe's options, each matching on every
 * attribute it names. Element handles select those `<option>` elements.
 */
export function selectOptionValues(
  api: string,
  values: unknown,
): {
  options: SelectOption[]
  elements: ElementBrand[]
} {
  if (values === null || values === undefined)
    return { options: [], elements: [] }
  const list: unknown[] = Array.isArray(values) ? values : [values]
  const options: ReturnType<typeof selectOptionValues>['options'] = []
  const elements: ElementBrand[] = []
  for (const [index, item] of list.entries()) {
    if (item === null)
      throw clientError(api, `options[${index}]: expected object, got null`)
    if (typeof item === 'string') {
      options.push({ valueOrLabel: item })
      continue
    }
    const brand: unknown =
      typeof item === 'object' ? Reflect.get(item, '_brand') : undefined
    if (
      handles.has(Object(item)) &&
      typeof brand === 'object' &&
      brand !== null
    ) {
      elements.push(brand as ElementBrand)
      continue
    }
    // Every attribute the script named goes through, as upstream's client sends the object:
    // the page selects the option that matches all of them.
    const option = item as SelectOptionValue
    const picked: SelectOption = {}
    if (option.valueOrLabel !== undefined)
      picked.valueOrLabel = String(option.valueOrLabel)
    if (option.value !== undefined) picked.value = String(option.value)
    if (option.label !== undefined) picked.label = String(option.label)
    if (option.index !== undefined) picked.index = Number(option.index)
    options.push(picked)
  }
  return { options, elements }
}
