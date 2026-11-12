/**
 * One run as Markdown, for an agent to read: the export's `runs/NNN.md` and the run row's Copy
 * render through this one function, so the two never drift. It is pure over the run's detail
 * (`GET /api/runs/:id`), and reads the run the way the page does: the code laid out by
 * `shared/code-format.ts` (whitespace only), Neo's operations placed on those lines, the reply's
 * parts and Neo time through the page's own helpers in `web/screen`. So a line number here is
 * the line on screen, and a number here is the number on screen.
 *
 * Order: a one-line header, the facts, the code, each code line's methods, (the export only:
 * every operation in a table, the ones on no line, the console lines), the error if it failed,
 * then what the model read.
 */

import { failed, neoMs } from '../web/screen/measures'
import {
  namesWithCounts,
  type PlacedOperations,
  placeOperations,
} from '../web/screen/operations'
import { noReplyReason, replyParts } from '../web/screen/reply'
import { clock, count, plural, seconds } from '../web/screen/units'
import type { NeoOperation, RunDetail, RunSummary } from './api'
import { textSize } from './api'
import { firstStatement, formatCode } from './code-format'

export interface RunMarkdownOptions {
  /** Characters of the reply (and of a failed run's error) shown inline; null shows all of it. */
  replyChars: number | null
  /** Where the full reply is (a link relative to the Markdown), named when only part is shown. */
  replyFile?: string
  /**
   * Every operation in a table, the ones not on a line by name, and the console lines (the
   * export). Otherwise one line per method on a code line and a count of the rest (Copy).
   */
  allOperations?: boolean
  /** Digits in the run number (`007`): the export pads every run to the widest. */
  numberWidth?: number
}

/** `ok`, `failed` (an error the model read, a JSON-RPC error, or an HTTP error), or the status. */
export function outcomeWord(run: RunSummary): string {
  if (run.status === 'ok') return 'ok'
  return failed(run) ? 'failed' : run.status
}

/** The run's number, zero-padded: `007`. */
export function runNumber(run: RunSummary, width = 3): string {
  return String(run.seq).padStart(width, '0')
}

/** The code's first statement on one line, as the page's closed row shows it, cut at `max`. */
export function statementPreview(code: string, max = 60): string {
  return cut(firstStatement(code), max)
}

/**
 * Exactly what the model read: the reply's text, or the JSON-RPC error it got instead; null when
 * it read nothing. The export writes this to the run's reply file.
 */
export function modelReadText(detail: RunDetail): string | null {
  const { reply } = detail
  return reply.text ?? (reply.rpcError ? JSON.stringify(reply.rpcError) : null)
}

/**
 * The run's returned value when it is a string, decoded for a person: Neo writes the value as one
 * line of JSON (`"- link [ref=e1]\n- …"`), so a returned snapshot reaches the model as escaped
 * `\n` and `\"`. `json` is that line as the model received it; `rest` the console lines and notes
 * after it, as read. Null for anything else (an error line, an object, no reply text).
 */
export function returnedString(
  detail: RunDetail,
): { value: string; json: string; rest: string } | null {
  const text = detail.reply.text
  if (text === null || detail.reply.isError) return null
  const parts = replyParts(text)
  let value: unknown
  try {
    value = JSON.parse(parts.value)
  } catch {
    return null
  }
  if (typeof value !== 'string') return null
  // The text after the value exactly as the model read it (console lines, notes), not rebuilt
  // from parsed pieces, which could take a console line for one of Neo's notes.
  const rest = text.slice(parts.value.length).replace(/^\n/, '')
  return { value, json: parts.value, rest }
}

/** The line that says a string is shown decoded, and what the model received instead. */
export function decodedNote(json: string): string {
  return `Shown decoded; the model received it as a JSON string of ${plural(json.length, 'character')}.`
}

/**
 * `text` in a fence no line of it can close: one backtick longer than the longest run of
 * backticks inside, at least three.
 */
export function fenced(text: string, info = ''): string {
  const longest = Math.max(
    0,
    ...[...text.matchAll(/`+/g)].map((match) => match[0].length),
  )
  const fence = '`'.repeat(Math.max(3, longest + 1))
  return `${fence}${info}\n${text}${text.endsWith('\n') ? '' : '\n'}${fence}`
}

/**
 * A Markdown table cell: one line, and every pipe escaped. A table splits a row at a pipe behind
 * an even number of backslashes (pairs escape each other), so the backslashes before a pipe are
 * doubled and one more added: the pipe stays in its cell even inside a code span.
 */
export function cell(text: string): string {
  return text
    .replace(/\r?\n/g, ' ')
    .replace(/(\\*)\|/g, (_, slashes: string) => `${slashes}${slashes}\\|`)
}

export function runMarkdown(
  detail: RunDetail,
  options: RunMarkdownOptions,
): string {
  const formatted = formatCode(detail.code)
  const placed = detail.neo
    ? placeOperations(detail.neo, formatted.lines.length, formatted.lineOfSite)
    : null
  const sections = [
    [`# ${runHeader(detail, options.numberWidth ?? 3)}`, '', ...facts(detail)],
    [
      '## Code',
      '',
      'Laid out one statement per line, as the viewer shows it (whitespace only); `L<n>` below is a line of it.',
      '',
      fenced(formatted.text, 'js'),
    ],
    ['## Methods by line', '', ...methodsByLine(placed, options)],
    ...(options.allOperations && detail.neo && placed
      ? everyOperation(detail.neo.operations, placed)
      : []),
    ...(options.allOperations ? consoleSection(detail, options) : []),
    ...errorSection(detail, options),
    ['## What the model read', '', ...modelRead(detail, options)],
  ]
  return `${sections.map((lines) => lines.join('\n')).join('\n\n')}\n`
}

/**
 * `Run 007 · 10:14:02 · claude-code · task shop-4b80 · ok · round trip 1.85 s · Neo 1.62 s ·
 * 4 method calls · code ≈52 tok · reply ≈220 tok`.
 */
function runHeader(run: RunSummary, width: number): string {
  const neo = neoMs(run)
  return [
    `Run ${runNumber(run, width)}`,
    clock(run.sentAt),
    run.client?.name ?? 'unknown client',
    ...taskWords(run),
    outcomeWord(run),
    `round trip ${run.roundTripMs === null ? '–' : seconds(run.roundTripMs)}`,
    `Neo ${neo === null ? '–' : seconds(neo)}`,
    methodCalls(run),
    `code ≈${count(run.codeSize.tokens)} tok`,
    `reply ≈${run.replySize ? count(run.replySize.tokens) : '–'} tok`,
  ].join(' · ')
}

/**
 * `4 method calls`; without Neo's record, why: Neo answered (it named the run) but its record of
 * the run had not been read in time, or there is nothing to know (no host API, no Neo run).
 */
function methodCalls(run: RunSummary): string {
  if (run.operationCount !== null)
    return plural(run.operationCount, 'method call')
  return run.neoRunId && run.status !== 'running'
    ? "Neo's record had not arrived"
    : 'method calls unknown'
}

/** `task shop-4b80`, `tasks a, b`, `host task h1`: what the run's tasks were, for the header. */
function taskWords(run: RunSummary): string[] {
  const words: string[] = []
  if (run.tasks.length > 0)
    words.push(
      `${run.tasks.length === 1 ? 'task' : 'tasks'} ${run.tasks.map((task) => task.id).join(', ')}`,
    )
  if (run.agentTaskId) words.push(`host task ${run.agentTaskId}`)
  return words
}

function facts(detail: RunDetail): string[] {
  const lines = [
    `- Sent ${detail.sentAt}${detail.answeredAt ? `, answered ${detail.answeredAt}` : ''}`,
  ]
  if (detail.gapMs !== null)
    lines.push(`- ${seconds(detail.gapMs)} after the previous reply`)
  if (detail.tasks.length > 0)
    lines.push(
      `- Tasks the script created or picked: ${detail.tasks
        .map(
          (task) =>
            `\`${task.id}\`${task.name ? ` (${inline(task.name)})` : ''}`,
        )
        .join(', ')}`,
    )
  if (detail.agentTaskId)
    lines.push(
      `- Host task (the request's \`_meta\` taskId): \`${detail.agentTaskId}\``,
    )
  if (detail.timeoutMs !== null)
    lines.push(`- Timeout asked for: ${seconds(detail.timeoutMs)}`)
  if (detail.neoRunId) {
    const status = detail.neo?.status ?? detail.neoStatus
    lines.push(
      `- Neo run \`${detail.neoRunId}\`${status ? `, status ${status}` : ''}${
        detail.neoEffects ? `, effects ${detail.neoEffects}` : ''
      }`,
    )
  }
  if (detail.neo && !detail.neo.complete)
    lines.push("- Neo's record was read before every operation had ended")
  return lines
}

/** `- L3 locator.click: +415 ms, 80 ms, ok` per method, then how many are on no line. */
function methodsByLine(
  placed: PlacedOperations | null,
  options: RunMarkdownOptions,
): string[] {
  if (!placed)
    return [
      'No record from Neo for this run (no host API, or it was not read yet), so no methods to show.',
    ]
  const out: string[] = []
  for (const line of [...placed.byLine.keys()].sort((a, b) => a - b))
    for (const operation of placed.byLine.get(line) ?? [])
      out.push(`- L${line} ${operation.name}: ${timing(operation)}`)
  if (out.length === 0) out.push('No method was called from a code line.')
  if (placed.unplaced.length > 0)
    out.push(
      '',
      `${plural(placed.unplaced.length, 'operation')} not on a code line${
        options.allOperations ? ' (below)' : ''
      }.`,
    )
  return out
}

/** `+415 ms, 80 ms, error strict_mode_violation`. */
function timing(operation: NeoOperation): string {
  const duration =
    operation.durationMs === null ? 'running' : seconds(operation.durationMs)
  const error = operation.error ? ` ${operation.error.code}` : ''
  return `+${seconds(operation.startOffsetMs)}, ${duration}, ${operation.outcome ?? 'running'}${error}`
}

const ERROR_CELL_CHARS = 160

/** The export's table of every operation, and the names of those on no code line. */
function everyOperation(
  operations: NeoOperation[],
  placed: PlacedOperations,
): string[][] {
  const lineOf = new Map<string, number>()
  for (const [line, onLine] of placed.byLine)
    for (const operation of onLine) lineOf.set(operation.id, line)
  const rows = [
    '| seq | name | effect | line | start | duration | outcome | error |',
    '| --: | --- | --- | --: | --: | --: | --- | --- |',
  ]
  for (const operation of [...operations].sort((a, b) => a.seq - b.seq)) {
    const error = operation.error
      ? `${operation.error.code}: ${operation.error.message}`
      : ''
    const duration =
      operation.durationMs === null ? 'running' : seconds(operation.durationMs)
    rows.push(
      `| ${operation.seq} | ${cell(operation.name)} | ${operation.effect} | ${
        lineOf.get(operation.id) ?? '–'
      } | +${seconds(operation.startOffsetMs)} | ${duration} | ${
        operation.outcome ?? 'running'
      } | ${cell(cut(error, ERROR_CELL_CHARS))} |`,
    )
  }
  const sections = [['## Operations', '', ...rows]]
  if (placed.unplaced.length > 0)
    sections.push([
      '## Operations not on a line',
      '',
      "Neo's own work for the run (opening the page, listening for events, reading frames) or calls made inside another operation:",
      '',
      namesWithCounts(placed.unplaced),
    ])
  return sections
}

/** The console lines, cut at the inline limit like the reply they are part of. */
function consoleSection(
  detail: RunDetail,
  options: RunMarkdownOptions,
): string[][] {
  if (detail.reply.text === null) return []
  const { console } = replyParts(detail.reply.text)
  return console.length === 0
    ? []
    : [['## Console', '', ...bounded(console.join('\n'), options)]]
}

/** Why the run failed, in Neo's words: the error the model read, Neo's own, or the transport's. */
function errorSection(
  detail: RunDetail,
  options: RunMarkdownOptions,
): string[][] {
  if (detail.status === 'ok' || detail.status === 'running') return []
  const { reply } = detail
  const parts: string[] = []
  if (reply.rpcError)
    parts.push(
      `JSON-RPC error ${reply.rpcError.code}: ${cut(reply.rpcError.message, options.replyChars)}`,
    )
  else if (reply.isError && reply.text !== null)
    parts.push(...bounded(replyParts(reply.text).value, options))
  else if (reply.text === null) parts.push(noReplyReason(detail))
  const neoError = detail.neo?.error
  // Neo's record usually holds the very error the model read; say it only when it adds something.
  if (neoError && !(reply.text ?? '').includes(neoError.message))
    parts.push(
      `Neo's record: ${neoError.owner} / ${neoError.code}: ${cut(neoError.message, options.replyChars)}${
        neoError.retryable ? ' (retryable)' : ''
      }`,
    )
  return parts.length > 0 ? [['## Error', '', parts.join('\n\n')]] : []
}

/**
 * What the model read. Its size is always what the model received. A returned string is shown
 * decoded, so a snapshot reads as lines when a person or an agent reads the Markdown; its exact
 * text stays in the Raw view, `runs/NNN.json` and the reply file.
 */
function modelRead(detail: RunDetail, options: RunMarkdownOptions): string[] {
  const text = modelReadText(detail)
  if (text === null) return [noReplyReason(detail)]
  const size = textSize(text)
  const sizeLine = `${plural(size.chars, 'char')}, ≈${count(size.tokens)} tok.`
  const decoded = returnedString(detail)
  if (!decoded) return [sizeLine, '', ...bounded(text, options)]
  return [
    sizeLine,
    '',
    decodedNote(decoded.json),
    '',
    ...bounded(decoded.value, options, 'decoded'),
    ...(decoded.rest
      ? [
          '',
          'Then, as the model read them:',
          '',
          ...bounded(decoded.rest, options),
        ]
      : []),
  ]
}

/**
 * `text` fenced, cut at the inline limit with a pointer to the file that holds all of it (for a
 * decoded string, the file holds the exact text the model received).
 */
function bounded(
  text: string,
  options: RunMarkdownOptions,
  decoded?: 'decoded',
): string[] {
  const limit = options.replyChars
  if (limit === null || text.length <= limit) return [fenced(text, 'text')]
  return [
    `The first ${count(limit)} characters${decoded ? ', decoded' : ''}${
      options.replyFile
        ? `; ${decoded ? 'the exact text the model received' : 'the full text'} is in [${options.replyFile}](${options.replyFile})`
        : ''
    }:`,
    '',
    fenced(text.slice(0, limit), 'text'),
  ]
}

/** A name from Neo, safe inline in Markdown: one line, its Markdown and HTML characters escaped. */
function inline(text: string): string {
  return text.replace(/\s+/g, ' ').replace(/[\\`*_[\]<>#|]/g, '\\$&')
}

function cut(text: string, max: number | null): string {
  return max === null || text.length <= max
    ? text
    : `${text.slice(0, max - 1)}…`
}
