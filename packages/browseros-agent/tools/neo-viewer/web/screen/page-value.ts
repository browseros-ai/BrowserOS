/**
 * An `evaluate` result as the script got it. Neo records the page's serialized form of the value
 * (`{ id, o: [{ k, v }] }` for an object, `{ v: 'undefined' }`, `{ d: … }` for a Date, …), and
 * the prelude turns it back into a value with Playwright's `parseEvaluationResultValue`. The
 * viewer runs that same vendored function on the recorded form, then prints the value it built:
 * JSON's layout for objects and arrays, and JavaScript's own words for what JSON cannot say
 * (`undefined`, `NaN`, `-0`, `Date`, `Map`, a typed array's numbers).
 *
 * A value can hold one object many times. It prints in full the first time; after that it is
 * `[the same object as above]`, and a reference back into an object still being printed is
 * `[Circular]`. So the text grows with the recorded value, never with how often its objects
 * repeat (a value of 784 bytes can otherwise print as megabytes).
 */

import { parseEvaluationResultValue } from '../../../../crates/neo-sdk/neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers'

const INDENT = '  '
/** A typed array shows this many of its numbers, then how many more. */
const TYPED_ITEMS = 64

/**
 * The script's value in words, from its recorded serialized form: a string as its own text.
 * Null when the run's decoder refuses the form (Neo cut it, say), so the caller says so.
 */
export function pageValueText(serialized: unknown): string | null {
  let value: unknown
  try {
    value = parseEvaluationResultValue(serialized as never)
  } catch {
    return null
  }
  if (typeof value === 'string') return value
  return inspect(value, '', { open: new Set(), printed: new Set() })
}

interface Seen {
  /** Objects being printed: a reference back to one is a cycle. */
  open: Set<object>
  /** Objects printed already. */
  printed: Set<object>
}

function inspect(value: unknown, indent: string, seen: Seen): string {
  if (typeof value !== 'object' || value === null) return primitive(value)
  if (seen.open.has(value)) return '[Circular]'
  if (seen.printed.has(value)) return '[the same object as above]'
  const leaf = leafObject(value)
  if (leaf !== null) return leaf
  seen.printed.add(value)
  seen.open.add(value)
  const text = collection(value, indent, seen)
  seen.open.delete(value)
  return text
}

function primitive(value: unknown): string {
  if (value === undefined) return 'undefined'
  if (typeof value === 'number')
    return Object.is(value, -0) ? '-0' : String(value)
  if (typeof value === 'bigint') return `${value}n`
  if (typeof value === 'function') return 'function'
  return JSON.stringify(value) ?? String(value)
}

/** Objects that print as one value: dates, URLs, patterns, errors, bytes. */
function leafObject(value: object): string | null {
  if (value instanceof Date) {
    return Number.isNaN(value.getTime())
      ? 'Invalid Date'
      : `Date ${JSON.stringify(value.toISOString())}`
  }
  if (value instanceof URL) return `URL ${JSON.stringify(value.href)}`
  if (value instanceof RegExp) return String(value)
  if (value instanceof Error) return `${value.name}: ${value.message}`
  if (value instanceof ArrayBuffer)
    return numbers('ArrayBuffer', new Uint8Array(value))
  if (ArrayBuffer.isView(value) && !(value instanceof DataView)) {
    return numbers(
      value.constructor.name,
      value as unknown as ArrayLike<unknown>,
    )
  }
  return null
}

function numbers(name: string, items: ArrayLike<unknown>): string {
  const shown = Array.from(
    { length: Math.min(items.length, TYPED_ITEMS) },
    (_, i) => primitive(items[i]),
  )
  const more = items.length - shown.length
  return `${name} [${shown.join(', ')}${more > 0 ? `, … ${more} more` : ''}]`
}

/** Arrays, Maps, Sets and plain objects, one item per line. */
function collection(value: object, indent: string, seen: Seen): string {
  const inner = indent + INDENT
  const show = (item: unknown) => inspect(item, inner, seen)
  if (Array.isArray(value)) return block('[', ']', value.map(show), indent)
  if (value instanceof Map) {
    const items = [...value].map(
      ([key, item]) => `${show(key)} => ${show(item)}`,
    )
    return block('Map {', '}', items, indent)
  }
  if (value instanceof Set)
    return block('Set {', '}', [...value].map(show), indent)
  const items = Object.entries(value).map(
    ([key, item]) => `${JSON.stringify(key)}: ${show(item)}`,
  )
  return block('{', '}', items, indent)
}

function block(
  open: string,
  close: string,
  items: string[],
  indent: string,
): string {
  if (items.length === 0) return `${open}${close}`
  const inner = indent + INDENT
  return `${open}\n${items.map((item) => `${inner}${item}`).join(',\n')}\n${indent}${close}`
}
