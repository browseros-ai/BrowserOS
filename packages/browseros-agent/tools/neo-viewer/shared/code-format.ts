/**
 * Display formatting for a `run()` script: agents send code minified on one line, and this lays
 * it out one statement per line, by changing whitespace only.
 *
 * Every token and comment is printed exactly as the agent sent it; only the gaps between them
 * change. So every original offset still points at the same character, and an operation's call
 * site (the script line and column Neo records, at the method name) maps to the display line that
 * now holds that token. That mapping is why this is not a reprinting formatter: Prettier and the
 * like rewrite tokens (quotes, parentheses, semicolons) and lose it.
 *
 *   code ──acorn──▶ tokens + comments, AST
 *        ──AST walk──▶ a decision per gap: break (an indent relative to an anchor token) or space
 *        ──render──▶ text; then, while a line is wider than the column, break one more construct
 *
 * Pure (no DOM, no React): the page renders it, and the server's export prints with it.
 */

import {
  type Analysis,
  analyse,
  type Candidate,
  type Decision,
} from './code-gaps'
import type { Item } from './code-tree'

/** The code with its display lines, and where each original position landed. */
export interface FormattedCode {
  /** The display text: the original tokens, laid out. */
  text: string
  lines: string[]
  /** The 1-based display line holding an original offset (0-based, UTF-16). */
  lineOfOffset(offset: number): number
  /** The 1-based display line of a site as Neo records it (1-based line and column); null outside the code. */
  lineOfSite(site: { line: number; column: number }): number | null
}

/** Wider lines get one more construct broken onto its own lines, while one is left to break. */
export const FORMAT_WIDTH = 80

/**
 * The script laid out for reading, or the script as sent (one line per original line) when it
 * does not parse. Whitespace inside strings, template literals, regexes and comments is never
 * touched, and no line is ever split inside them.
 */
export function formatCode(
  code: string,
  width: number = FORMAT_WIDTH,
): FormattedCode {
  return layout(code, width) ?? asSent(code)
}

/** The first statement on one line (spaces as formatted, no breaks), for a closed row. */
export function firstStatement(code: string): string {
  const parsed = analyse(code)
  const first = parsed?.program.body.find(
    (node) => node.type !== 'EmptyStatement',
  )
  if (!parsed || !first) {
    return (
      code
        .split('\n')
        .find((line) => line.trim() !== '')
        ?.trim() ?? ''
    )
  }
  const { items, decisions } = parsed
  let text = ''
  for (let i = 0; i < items.length; i++) {
    const item = items[i] as Item
    if (item.start < first.start) continue
    if (item.end > first.end) break
    if (text !== '') {
      const previous = items[i - 1] as Item
      if (previous.kind === 'line-comment') break
      const gap = code.slice(previous.end, item.start)
      const decision = decisions.get(i)
      text +=
        gap === '' && decision === undefined
          ? ''
          : gap === '' || decision?.kind === 'break' || gap.includes('\n')
            ? ' '
            : gap
    }
    text +=
      item.kind === 'block-comment' ? '' : item.text.replace(/\n\s*/g, ' ')
  }
  return text.trim()
}

/** One display line per original line: the fallback when the code does not parse. */
function asSent(code: string): FormattedCode {
  // Trailing blank lines only: earlier lines keep their numbers, so sites still match.
  const lines = code.trimEnd().split('\n')
  const starts = lineStarts(code)
  const lineOfOffset = (offset: number) => {
    let line = 1
    while (line < starts.length && (starts[line] as number) <= offset) line++
    return line
  }
  return {
    text: lines.join('\n'),
    lines,
    lineOfOffset,
    lineOfSite: (site) =>
      site.line >= 1 && site.line <= lines.length ? site.line : null,
  }
}

function lineStarts(code: string): number[] {
  const starts = [0]
  for (let i = 0; i < code.length; i++) {
    if (code[i] === '\n') starts.push(i + 1)
  }
  return starts
}

function layout(code: string, width: number): FormattedCode | null {
  const analysis = analyse(code)
  if (!analysis) return null
  const { items, decisions, candidates } = analysis
  const applied = new Set<Candidate>()
  let rendered = render(code, analysis)
  // On the first too-wide line that still has something to break, break its outermost
  // construct (the kinds' order breaks ties), then lay out again.
  for (let round = 0; round < 200; round++) {
    const open = candidates.filter((each) => !applied.has(each))
    const line = rendered.lines.findIndex(
      (text, index) =>
        text.length > width &&
        open.some((each) => rendered.itemLine[each.first] === index + 1),
    )
    if (line === -1) break
    const onLine = open
      .filter((each) => rendered.itemLine[each.first] === line + 1)
      .sort((a, b) => b.size - a.size || a.rank - b.rank)
    applyCandidate(onLine[0] as Candidate, decisions, applied)
    rendered = render(code, analysis)
  }
  const { text, lines, itemLine } = rendered
  const starts = lineStarts(code)
  const lineOfOffset = (offset: number) => {
    const index = Math.min(
      Math.max(0, upperBound(items, offset) - 1),
      items.length - 1,
    )
    const item = items[index]
    if (!item) return 1
    if (offset >= item.end) {
      // In a gap: the next item's line.
      return itemLine[Math.min(index + 1, items.length - 1)] ?? 1
    }
    const inside = code.slice(item.start, offset).split('\n').length - 1
    return (itemLine[index] ?? 1) + inside
  }
  return {
    text,
    lines,
    lineOfOffset,
    lineOfSite: (site) => {
      const start = starts[site.line - 1]
      if (start === undefined || site.column < 1) return null
      const offset = start + site.column - 1
      return offset < code.length ? lineOfOffset(offset) : null
    },
  }
}

function applyCandidate(
  candidate: Candidate,
  decisions: Map<number, Decision>,
  applied: Set<Candidate>,
) {
  applied.add(candidate)
  for (const [index, decision] of candidate.breaks) {
    if (index > 0) decisions.set(index, decision)
  }
}

/** Items with their gaps decided: the text, its lines, and each item's 1-based display line. */
function render(code: string, analysis: Analysis) {
  const { items, decisions, unit } = analysis
  const lineIndent: number[] = []
  const itemLine: number[] = []
  let text = ''
  let line = 1
  let indent = 0
  for (let i = 0; i < items.length; i++) {
    const item = items[i] as Item
    if (i > 0) {
      const previous = items[i - 1] as Item
      const gap = code.slice(previous.end, item.start)
      const decision = decisions.get(i)
      let written: string
      if (previous.kind === 'line-comment' && decision?.kind !== 'break') {
        // A line comment runs to the end of its line: what follows keeps its newline.
        written = gap
      } else if (decision?.kind === 'break') {
        const base =
          decision.anchor === null ? 0 : (lineIndent[decision.anchor] ?? 0)
        written = `\n${' '.repeat(base + unit * decision.extra)}`
      } else if (decision?.kind === 'space') {
        written = gap === '' ? ' ' : gap
      } else {
        written = gap
      }
      const newlines = written.split('\n').length - 1
      if (newlines > 0) {
        line += newlines
        indent = written.length - written.lastIndexOf('\n') - 1
      }
      text += written
    }
    lineIndent[i] = indent
    itemLine[i] = line
    text += item.text
    line += item.text.split('\n').length - 1
  }
  return { text, lines: text.split('\n'), itemLine }
}

/** The index of the first item starting after `offset`. */
function upperBound(items: Item[], offset: number): number {
  let low = 0
  let high = items.length
  while (low < high) {
    const mid = (low + high) >> 1
    if ((items[mid] as Item).start <= offset) low = mid + 1
    else high = mid
  }
  return low
}
