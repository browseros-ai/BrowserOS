/**
 * An operation's arguments as its detail under the row shows them. Neo records arguments as
 * structured JSON: a locator or page is a handle (`{"$neo":"locator","display":"getByRole('button')"}`)
 * and the call's own settings sit under `call` (`{api, timeoutMs}`). The panel shows a handle by
 * its `display`, the timeout as `timeout`, and, when the script called the method by another name
 * (`page.waitForEvent` recorded as `events.wait`), that name as `called as`. Everything else is
 * the recorded JSON, with any long string cut to its start: the viewer asks Neo to keep values
 * whole (the host setting `trace.valueBytes`), so one argument can hold a whole snapshot's text, which the
 * right panel shows in full.
 */

import { jsonLine, type Line, type Segment } from './text-lines'

const INLINE_JSON = 60
/** A string argument longer than this shows its start and how much more there is. */
const ARGUMENT_TEXT = 200

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/**
 * A Neo handle in words: a locator or frame by its `display`, a page by its tab. Other tagged
 * values (`{"$neo":"bytes","base64":…}`, a blob marker) are data, shown as recorded; null then.
 */
export function handleText(value: unknown): string | null {
  if (!isRecord(value) || typeof value.$neo !== 'string') return null
  if (typeof value.display === 'string') return value.display
  if (value.$neo === 'page' && typeof value.tab === 'number') {
    return `page (tab ${value.tab})`
  }
  return null
}

/**
 * Handles anywhere inside a value replaced by their words (a screenshot's `mask` locators), and
 * long strings by their start.
 */
function simplified(value: unknown): unknown {
  const handle = handleText(value)
  if (handle !== null) return handle
  if (typeof value === 'string' && value.length > ARGUMENT_TEXT) {
    return `${value.slice(0, ARGUMENT_TEXT)}… (${value.length - ARGUMENT_TEXT} more chars)`
  }
  if (Array.isArray(value)) return value.map(simplified)
  if (isRecord(value)) {
    return Object.fromEntries(
      Object.entries(value).map(([key, item]) => [key, simplified(item)]),
    )
  }
  return value
}

function valueSegments(value: unknown, indent: string): Line[] {
  const handle = handleText(value)
  if (handle !== null) return [[{ text: handle, tone: 'plain' }]]
  const data = simplified(value)
  const compact = JSON.stringify(data) ?? 'undefined'
  if (compact.length <= INLINE_JSON) return [jsonLine(compact)]
  const [first = '', ...rest] = (
    JSON.stringify(data, null, 2) ?? 'undefined'
  ).split('\n')
  return [jsonLine(first), ...rest.map((line) => jsonLine(indent + line))]
}

/** Keys aligned beside their values, as the artboard's Arguments column. */
function keyed(entries: [string, unknown][]): Line[] {
  const width =
    entries.reduce((widest, [key]) => Math.max(widest, key.length), 0) + 1
  const indent = ' '.repeat(width)
  const lines: Line[] = []
  for (const [key, value] of entries) {
    const label: Segment = { text: key.padEnd(width), tone: 'key' }
    const [first = [], ...rest] = valueSegments(value, indent)
    lines.push([label, ...first], ...rest)
  }
  return lines
}

export function argumentLines(name: string, args: unknown): Line[] {
  if (args === null || args === undefined) return []
  if (!isRecord(args)) return valueSegments(args, '')
  const entries: [string, unknown][] = []
  let timeout: unknown
  let calledAs: unknown
  for (const [key, value] of Object.entries(args)) {
    if (key === 'call' && isRecord(value)) {
      timeout = value.timeoutMs
      calledAs = value.api
      continue
    }
    entries.push([key, value])
  }
  if (typeof calledAs === 'string' && calledAs !== name) {
    entries.unshift(['called as', { $neo: 'name', display: calledAs }])
  }
  if (typeof timeout === 'number') entries.push(['timeout', timeout])
  return keyed(entries)
}
