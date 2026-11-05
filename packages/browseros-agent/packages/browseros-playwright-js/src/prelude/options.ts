/**
 * Turning a script's option bags and arguments into wire fields.
 *
 * playwright-core's protocol validator drops option keys it does not know, so a script may pass
 * extra ones. Rust decodes with `deny_unknown_fields`, so each op sends only the keys its wire
 * type names (`pick`). Evaluate arguments travel as JSON in v1; a Playwright handle passed as an
 * argument is refused here with what to call instead, before the bootstrap's codec would refuse
 * it as an unbranded object.
 */
import { clientError, unsupportedError } from './errors'
import type { ElementBrand, Evaluation } from './wire'

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
  'steps',
  'trial',
] as const
export const DBLCLICK_KEYS = POINTER_KEYS.filter((key) => key !== 'clickCount')
export const HOVER_KEYS = [
  'force',
  'modifiers',
  'noWaitAfter',
  'position',
  'trial',
] as const
export const CHECK_KEYS = ['force', 'noWaitAfter', 'position', 'trial'] as const
export const DRAG_KEYS = [
  'force',
  'noWaitAfter',
  'sourcePosition',
  'targetPosition',
  'steps',
  'trial',
] as const
export const FILL_KEYS = ['force', 'noWaitAfter'] as const
export const KEY_KEYS = ['delay', 'noWaitAfter'] as const
/** The screenshot options Rust captures with; frame.ts refuses the rest. */
export const SCREENSHOT_KEYS = ['type', 'quality', 'fullPage', 'clip'] as const

/** Objects the prelude hands to scripts (pages, frames, locators, handles), by identity. */
const handles = new WeakSet<object>()

export function markHandle(object: object): void {
  handles.add(object)
}

function findHandle(value: unknown, seen: Set<unknown>): boolean {
  if (typeof value !== 'object' || value === null || seen.has(value))
    return false
  if (handles.has(value)) return true
  seen.add(value)
  for (const item of Object.values(value))
    if (findHandle(item, seen)) return true
  return false
}

/** A main-world call's wire form; an absent `arg` reaches the function as `undefined`. */
export function evaluation(
  api: string,
  pageFunction: unknown,
  arg: unknown,
): Evaluation {
  if (typeof pageFunction !== 'function' && typeof pageFunction !== 'string')
    throw clientError(api, 'expected a function or an expression string')
  if (findHandle(arg, new Set()))
    throw unsupportedError(
      api,
      'Neo passes evaluate arguments as JSON, without handles; run the function on the element itself with locator.evaluate(fn) or elementHandle.evaluate(fn)',
    )
  const call: Evaluation = {
    expression: String(pageFunction),
    isFunction: typeof pageFunction === 'function',
  }
  if (arg !== undefined) call.arg = arg
  return call
}

/** `locator.setInputFiles` values as host paths. */
export function inputFilePaths(api: string, files: unknown): string[] {
  const list: unknown[] = Array.isArray(files) ? files : [files]
  if (list.every((file) => typeof file === 'string')) return list as string[]
  throw unsupportedError(
    api,
    'Neo sets files from paths on the host; pass file paths, not { name, mimeType, buffer } payloads',
  )
}

type SelectOptionValue = {
  value?: string
  label?: string
  index?: number
  valueOrLabel?: string
}

/**
 * upstream's convertSelectOptionValues, onto the probe's options, which match on one of value,
 * label or index each. Element handles select those `<option>` elements.
 */
export function selectOptionValues(
  api: string,
  values: unknown,
): {
  options: (
    | { valueOrLabel: string }
    | { value: string }
    | { label: string }
    | { index: number }
  )[]
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
    const option = item as SelectOptionValue
    const given = (['valueOrLabel', 'value', 'label', 'index'] as const).filter(
      (key) => option[key] !== undefined,
    )
    if (given.length !== 1)
      throw clientError(
        api,
        `options[${index}]: Neo matches an option by one of value, label or index; got ${JSON.stringify(item)}`,
      )
    const [key] = given
    if (key === 'index') options.push({ index: Number(option.index) })
    else if (key === 'value') options.push({ value: String(option.value) })
    else if (key === 'label') options.push({ label: String(option.label) })
    else options.push({ valueOrLabel: String(option.valueOrLabel) })
  }
  return { options, elements }
}
