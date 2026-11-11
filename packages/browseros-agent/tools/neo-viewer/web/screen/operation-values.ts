/**
 * An operation's arguments and result as its detail panel shows them. Neo records arguments as
 * structured JSON: a locator or page is a handle (`{"$neo":"locator","display":"getByRole('button')"}`)
 * and the call's own settings sit under `call` (`{api, timeoutMs}`). The panel shows a handle by
 * its `display`, the timeout as `timeout`, and, when the script called the method by another name
 * (`page.waitForEvent` recorded as `events.wait`), that name as `called as`. Everything else is
 * the recorded JSON.
 */

import { jsonLine, type Line, type Segment, snapshotLine } from './text-lines'

const INLINE_JSON = 60

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

/** A Neo handle in words, or null when the value is not one. */
export function handleText(value: unknown): string | null {
  if (!isRecord(value) || typeof value.$neo !== 'string') return null
  if (typeof value.display === 'string') return value.display
  if (typeof value.tab === 'number') return `${value.$neo} (tab ${value.tab})`
  return value.$neo
}

function valueSegments(value: unknown, indent: string): Line[] {
  const handle = handleText(value)
  if (handle !== null) return [[{ text: handle, tone: 'plain' }]]
  const compact = JSON.stringify(value) ?? 'undefined'
  if (compact.length <= INLINE_JSON) return [jsonLine(compact)]
  const [first = '', ...rest] = (
    JSON.stringify(value, null, 2) ?? 'undefined'
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

/** The recorded result: text line by line, anything else as pretty JSON. */
export function resultLines(result: unknown): Line[] {
  if (result === undefined) return []
  if (typeof result === 'string' && result.includes('\n')) {
    return result.split('\n').map(snapshotLine)
  }
  const handle = handleText(result)
  if (handle !== null) return [[{ text: handle, tone: 'plain' }]]
  return (JSON.stringify(result, null, 2) ?? 'undefined')
    .split('\n')
    .map(jsonLine)
}
