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

export const FILL_KEYS = ['force'] as const
export const PRESS_KEYS = ['delay', 'noWaitAfter'] as const
export const TYPE_KEYS = ['delay'] as const
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
  'maskColor',
] as const

/** Objects the prelude hands to scripts (pages, frames, locators, handles), by identity. */
const handles = new WeakSet<object>()

export function markHandle(object: object): void {
  handles.add(object)
}

/**
 * Node's `util.inspect.custom`, which is this registry symbol. Upstream's client gives
 * `ConsoleMessage` and `Locator` one, so `util.inspect(message)` prints the message's text and
 * a locator its description; a realm without Node's `util` never reads it.
 */
export const inspectCustom = Symbol.for('nodejs.util.inspect.custom')

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
 * One option object, checked as upstream's protocol validator checks a `SelectOption`
 * (validatorPrimitives.ts `tObject`, `tString`, `tInt`, with its messages), keeping every
 * attribute it names: the page selects the option matching all of them. An object naming none
 * matches any option, as upstream's does; anything else is refused before it is sent.
 */
function selectOption(api: string, path: string, item: unknown): SelectOption {
  if (typeof item !== 'object' || item === null)
    throw clientError(
      api,
      `${path}: expected object, got ${item === null ? 'null' : typeof item}`,
    )
  const option = item as SelectOptionValue
  const picked: SelectOption = {}
  for (const key of ['valueOrLabel', 'value', 'label'] as const) {
    const given: unknown = option[key]
    if (given === undefined) continue
    if (given instanceof String) picked[key] = given.valueOf()
    else if (typeof given === 'string') picked[key] = given
    else
      throw clientError(
        api,
        `${path}.${key}: expected string, got ${typeof given}`,
      )
  }
  const index: unknown = option.index
  if (index !== undefined) {
    const value =
      index instanceof Number
        ? index.valueOf()
        : typeof index === 'number'
          ? index
          : null
    if (value === null)
      throw clientError(
        api,
        `${path}.index: expected integer, got ${typeof index}`,
      )
    if (!Number.isInteger(value))
      throw clientError(
        api,
        `${path}.index: expected integer, got float ${value}`,
      )
    picked.index = value
  }
  return picked
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
    options.push(selectOption(api, `options[${index}]`, item))
  }
  return { options, elements }
}
