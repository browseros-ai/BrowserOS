/**
 * Neo's reply sections: Result (one JSON line) or Error, then Console, Notes and Tips when
 * present. Headers match whole lines. Neo escapes matching console/error lines with a
 * backslash, so their text cannot start a section. Older replies remain whole as the value.
 */

import type { RunDetail, TextSize } from '../../shared/api'
import {
  jsonLine,
  type Line,
  plainLines,
  type Segment,
  snapshotLine,
} from './text-lines'
import { count, plural } from './units'

export interface ReplyParts {
  /** The value's JSON line, or the error line(s). */
  value: string
  console: string[]
  /** Neo's notes on operations the script left unconfirmed. */
  notes: string[]
  tips: string[]
}

const SECTIONS = new Map<string, keyof ReplyParts>([
  ['### Result', 'value'],
  ['### Error', 'value'],
  ['### Console', 'console'],
  ['### Notes', 'notes'],
  ['### Tips', 'tips'],
])

export function replyParts(text: string): ReplyParts {
  const parts: Record<keyof ReplyParts, string[]> = {
    value: [],
    console: [],
    notes: [],
    tips: [],
  }
  let section: keyof ReplyParts | undefined
  for (const line of text.split('\n')) {
    const header = SECTIONS.get(line)
    if (header !== undefined) {
      section = header
    } else if (section !== undefined) {
      const escaped =
        (section === 'value' || section === 'console') &&
        line.startsWith('\\') &&
        SECTIONS.has(line.slice(1))
      parts[section].push(escaped ? line.slice(1) : line)
    }
  }
  return {
    ...parts,
    value: section === undefined ? text : parts.value.join('\n'),
  }
}

/** `owner / code: message`, Neo's error line; the code names the failure in a closed row. */
export function errorCode(text: string | null): string | null {
  const match =
    text === null ? null : replyParts(text).value.match(/^[\w-]+ \/ ([\w.-]+):/)
  return match?.[1] ?? null
}

function parseValue(value: string): { ok: true; value: unknown } | null {
  try {
    return { ok: true, value: JSON.parse(value) }
  } catch {
    return null
  }
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return typeof value === 'object' && value !== null && !Array.isArray(value)
}

function lineCount(text: string): number {
  return text.split('\n').length
}

/**
 * The formatted reply: an object's keys aligned beside their values, a multi-line string (an
 * aria snapshot) under its key line by line, then the console lines, notes and tips.
 */
export function formattedLines(detail: RunDetail): Line[] {
  const { reply } = detail
  if (reply.rpcError) {
    return [
      [{ text: 'JSON-RPC error', tone: 'error' }],
      ...JSON.stringify(reply.rpcError, null, 2).split('\n').map(jsonLine),
    ]
  }
  if (reply.text === null) return []
  const parts = replyParts(reply.text)
  const lines: Line[] = reply.isError
    ? plainLines(parts.value, 'error')
    : valueLines(parts.value)
  if (parts.console.length > 0) {
    lines.push([{ text: 'console', tone: 'muted' }])
    for (const line of parts.console) {
      lines.push([
        {
          text: line,
          tone: line.startsWith('[error] ')
            ? 'error'
            : line.startsWith('[warn] ')
              ? 'note'
              : 'plain',
        },
      ])
    }
  }
  for (const note of parts.notes) lines.push([{ text: note, tone: 'note' }])
  for (const tip of parts.tips) lines.push([{ text: tip, tone: 'note' }])
  return lines
}

function valueLines(value: string): Line[] {
  const parsed = parseValue(value)
  if (!parsed) return value.split('\n').map(snapshotLine)
  const data = parsed.value
  if (typeof data === 'string') return data.split('\n').map(snapshotLine)
  if (!isRecord(data) || Object.keys(data).length === 0) {
    return JSON.stringify(data, null, 2).split('\n').map(jsonLine)
  }
  const keys = Object.keys(data)
  const width =
    keys.reduce((widest, key) => Math.max(widest, key.length), 0) + 1
  const lines: Line[] = []
  for (const key of keys) {
    const item = data[key]
    if (typeof item === 'string' && item.includes('\n')) {
      lines.push([{ text: key, tone: 'key' }])
      // A loop, not push(...lines): a snapshot can have more lines than a call takes arguments.
      for (const text of item.split('\n')) lines.push(snapshotLine(text))
      continue
    }
    const json = JSON.stringify(item, null, 2) ?? 'undefined'
    const [first = '', ...rest] = json.split('\n')
    const label: Segment = { text: key.padEnd(width), tone: 'key' }
    lines.push([label, ...jsonLine(first)])
    const indent = ' '.repeat(width)
    for (const more of rest) lines.push(jsonLine(indent + more))
  }
  return lines
}

/** The reply exactly as the model read it. */
export function rawLines(detail: RunDetail): Line[] {
  const { reply } = detail
  if (reply.text !== null) return plainLines(reply.text)
  if (reply.rpcError) return plainLines(JSON.stringify(reply.rpcError))
  return []
}

/** What the model read when the run has no reply text, in words. */
export function noReplyReason(detail: RunDetail): string {
  switch (detail.status) {
    case 'running':
      return "Waiting for Neo's reply."
    case 'cancelled':
      return 'The agent went away before the reply; the viewer stopped the run.'
    case 'lost':
      return 'The viewer stopped before the reply arrived.'
    case 'http-error':
      return `Neo answered HTTP ${detail.reply.httpStatus ?? '–'} without a reply.`
    default:
      return 'No reply text.'
  }
}

/** `890 chars, ≈222 tok, no console lines`: the facts under a reply, sized by the API. */
export function replyFacts(text: string, size: TextSize): string {
  const { console } = replyParts(text)
  return `${count(size.chars)} chars, ≈${count(size.tokens)} tok, ${
    console.length === 0
      ? 'no console lines'
      : plural(console.length, 'console line')
  }`
}

const SHORT_VALUE = 48

/**
 * The reply's value in one short chip: short values verbatim, an object as its keys (a
 * multi-line string as its line count), an array as its length.
 */
export function valueChip(text: string): { text: string; failed: boolean } {
  const { value } = replyParts(text)
  const parsed = parseValue(value)
  if (!parsed) return { text: value.split('\n')[0] ?? '', failed: true }
  if (value.length <= SHORT_VALUE) return { text: value, failed: false }
  const data = parsed.value
  if (Array.isArray(data))
    return { text: `[ ${plural(data.length, 'item')} ]`, failed: false }
  if (typeof data === 'string') {
    return {
      text: data.includes('\n')
        ? `"${data.split('\n')[0]?.slice(0, 24)}…", ${lineCount(data)} lines`
        : `"${data.slice(0, SHORT_VALUE - 4)}…"`,
      failed: false,
    }
  }
  if (isRecord(data)) {
    const keys = Object.entries(data).map(([key, item]) =>
      typeof item === 'string' && item.includes('\n')
        ? `${key}: ${lineCount(item)} lines`
        : key,
    )
    return { text: `{ ${keys.join(', ')} }`, failed: false }
  }
  return { text: value, failed: false }
}

/** `returned 2 values`, `returned a 25-line string`, `failed: strict_mode_violation`. */
export function returnedWhat(detail: RunDetail): string | null {
  const { text, isError } = detail.reply
  if (text === null) return null
  const parts = replyParts(text)
  if (isError) return `failed: ${errorCode(text) ?? 'error'}`
  const parsed = parseValue(parts.value)
  if (!parsed) return null
  const data = parsed.value
  if (data === null) return 'returned null'
  if (Array.isArray(data)) return `returned ${plural(data.length, 'item')}`
  if (isRecord(data))
    return `returned ${plural(Object.keys(data).length, 'value')}`
  if (typeof data === 'string' && data.includes('\n')) {
    return `returned a ${lineCount(data)}-line string`
  }
  return parts.value.length <= SHORT_VALUE
    ? `returned ${parts.value}`
    : 'returned a value'
}
