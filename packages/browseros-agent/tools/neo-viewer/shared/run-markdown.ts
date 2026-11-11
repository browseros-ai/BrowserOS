/**
 * One run as Markdown, for an agent to read: the export's `runs/NNN.md` and the run row's Copy
 * render through this one function, so the two never drift. It is pure over the run's detail
 * (`GET /api/runs/:id`), and reads the run the way the page does, through the page's own pure
 * helpers in `web/screen` (operations placed on lines, the reply's parts, Neo time), so a number
 * here is the number on screen.
 *
 * Order: a one-line header, the facts, the code as sent, each code line's methods, (the export
 * only: every operation in a table, the ones not on a line, the console lines), the error if it
 * failed, then what the model read.
 */

import { failed, neoMs } from '../web/screen/measures'
import { namesWithCounts, placeOperations } from '../web/screen/operations'
import { noReplyReason, replyParts } from '../web/screen/reply'
import { clock, count, plural, seconds } from '../web/screen/units'
import type { NeoOperation, RunDetail, RunSummary } from './api'
import { textSize } from './api'

export interface RunMarkdownOptions {
  /** Characters of the reply shown inline; null shows all of it. */
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

/** The code's first statement on one line: its first line that is not blank or a comment. */
export function firstStatement(code: string, max = 60): string {
  const line =
    code
      .split('\n')
      .map((text) => text.trim())
      .find((text) => text !== '' && !text.startsWith('//')) ?? ''
  return line.length > max ? `${line.slice(0, max - 1)}…` : line
}

/** `Neo 1.62 s`: Neo's record of the run, or what the viewer measured without one. */
function neoTime(run: RunSummary): string {
  const ms = neoMs(run)
  return ms === null ? '–' : seconds(ms)
}

/**
 * `Run 007 · 10:14:02 · claude-code · ok · round trip 1.85 s · Neo 1.62 s · 4 method calls ·
 * code ≈52 tok · reply ≈220 tok`.
 */
function runHeader(run: RunSummary, width = 3): string {
  const parts = [
    `Run ${runNumber(run, width)}`,
    clock(run.sentAt),
    run.client?.name ?? 'unknown client',
    outcomeWord(run),
    `round trip ${run.roundTripMs === null ? '–' : seconds(run.roundTripMs)}`,
    `Neo ${neoTime(run)}`,
    run.operationCount === null
      ? 'method calls unknown'
      : plural(run.operationCount, 'method call'),
    `code ≈${count(run.codeSize.tokens)} tok`,
    `reply ≈${run.replySize ? count(run.replySize.tokens) : '–'} tok`,
  ]
  return parts.join(' · ')
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

/** A Markdown table cell: one line, pipes escaped. */
export function cell(text: string): string {
  return text.replace(/\r?\n/g, ' ').replace(/\|/g, '\\|')
}

export function runMarkdown(
  detail: RunDetail,
  options: RunMarkdownOptions,
): string {
  const width = options.numberWidth ?? 3
  const out: string[] = [`# ${runHeader(detail, width)}`, '', ...facts(detail)]
  out.push('', '## Code', '', fenced(detail.code, 'js'))
  const lineCount = detail.code.split('\n').length
  out.push('', '## Methods by line', '')
  if (!detail.neo) {
    out.push(
      'No record from Neo for this run (no host API, or it was not read yet), so no methods to show.',
    )
  } else {
    const placed = placeOperations(detail.neo, lineCount)
    const lines = [...placed.byLine.keys()].sort((a, b) => a - b)
    if (lines.length === 0) out.push('No method was called from a code line.')
    for (const line of lines)
      for (const operation of placed.byLine.get(line) ?? [])
        out.push(`- L${line} ${operation.name}: ${timing(operation)}`)
    if (placed.unplaced.length > 0) {
      out.push(
        '',
        `${plural(placed.unplaced.length, 'operation')} not on a code line${
          options.allOperations ? ' (below)' : ''
        }.`,
      )
    }
    if (options.allOperations) {
      out.push(
        '',
        '## Operations',
        '',
        ...operationTable(detail.neo.operations),
      )
      if (placed.unplaced.length > 0) {
        out.push(
          '',
          '## Operations not on a line',
          '',
          "Neo's own work for the run (opening the page, listening for events, reading frames) or calls made inside another operation:",
          '',
          namesWithCounts(placed.unplaced),
        )
      }
    }
  }
  if (options.allOperations && detail.reply.text !== null) {
    const { console } = replyParts(detail.reply.text)
    if (console.length > 0)
      out.push('', '## Console', '', fenced(console.join('\n'), 'text'))
  }
  const error = errorText(detail)
  if (error) out.push('', '## Error', '', error)
  out.push('', '## What the model read', '', ...modelRead(detail, options))
  return `${out.join('\n')}\n`
}

function facts(detail: RunDetail): string[] {
  const lines = [
    `- Sent ${detail.sentAt}${detail.answeredAt ? `, answered ${detail.answeredAt}` : ''}`,
  ]
  if (detail.gapMs !== null)
    lines.push(`- ${seconds(detail.gapMs)} after the previous reply`)
  if (detail.agentTaskId)
    lines.push(`- Agent task id: \`${detail.agentTaskId}\``)
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

/** `+415 ms, 80 ms, error strict_mode_violation`. */
function timing(operation: NeoOperation): string {
  const duration =
    operation.durationMs === null ? 'running' : seconds(operation.durationMs)
  const outcome = operation.outcome ?? 'running'
  const error = operation.error ? ` ${operation.error.code}` : ''
  return `+${seconds(operation.startOffsetMs)}, ${duration}, ${outcome}${error}`
}

const ERROR_CELL_CHARS = 160

function operationTable(operations: NeoOperation[]): string[] {
  const rows = [
    '| seq | name | effect | line | start | duration | outcome | error |',
    '| --: | --- | --- | --: | --: | --: | --- | --- |',
  ]
  for (const operation of [...operations].sort((a, b) => a.seq - b.seq)) {
    const error = operation.error
      ? `${operation.error.code}: ${operation.error.message}`
      : ''
    rows.push(
      `| ${operation.seq} | ${cell(operation.name)} | ${operation.effect} | ${
        operation.site?.line ?? '–'
      } | +${seconds(operation.startOffsetMs)} | ${
        operation.durationMs === null
          ? 'running'
          : seconds(operation.durationMs)
      } | ${operation.outcome ?? 'running'} | ${cell(
        error.length > ERROR_CELL_CHARS
          ? `${error.slice(0, ERROR_CELL_CHARS - 1)}…`
          : error,
      )} |`,
    )
  }
  return rows
}

/** Why the run failed, in Neo's words: the error the model read, Neo's own, or the transport's. */
function errorText(detail: RunDetail): string | null {
  if (!failed(detail) && detail.status === 'ok') return null
  const parts: string[] = []
  const { reply } = detail
  if (reply.rpcError)
    parts.push(
      `JSON-RPC error ${reply.rpcError.code}: ${reply.rpcError.message}`,
    )
  else if (reply.isError && reply.text !== null)
    parts.push(fenced(replyParts(reply.text).value, 'text'))
  else if (reply.text === null) parts.push(noReplyReason(detail))
  const neoError = detail.neo?.error
  // Neo's record usually holds the very error the model read; say it only when it adds something.
  if (neoError && !(reply.text ?? '').includes(neoError.message))
    parts.push(
      `Neo's record: ${neoError.owner} / ${neoError.code}: ${neoError.message}${
        neoError.retryable ? ' (retryable)' : ''
      }`,
    )
  return parts.length > 0 ? parts.join('\n\n') : null
}

function modelRead(detail: RunDetail, options: RunMarkdownOptions): string[] {
  const { reply } = detail
  const text =
    reply.text ?? (reply.rpcError ? JSON.stringify(reply.rpcError) : null)
  if (text === null) return [noReplyReason(detail)]
  const size = textSize(text)
  const sizeLine = `${plural(size.chars, 'char')}, ≈${count(size.tokens)} tok.`
  const limit = options.replyChars
  if (limit === null || text.length <= limit)
    return [sizeLine, '', fenced(text, 'text')]
  return [
    `${sizeLine} The first ${count(limit)} characters${
      options.replyFile
        ? `; the full text is in [${options.replyFile}](${options.replyFile})`
        : ''
    }.`,
    '',
    fenced(text.slice(0, limit), 'text'),
  ]
}
