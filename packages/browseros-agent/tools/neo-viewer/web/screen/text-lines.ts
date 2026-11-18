/**
 * Text as the screen draws it: lines of toned segments. The code block, the reply sidebar and the
 * full-output pop-up all render `Line[]`, so search and copy work on the same text that is shown.
 */

export type Tone =
  | 'plain'
  | 'muted'
  | 'key'
  | 'string'
  | 'ref'
  | 'keyword'
  | 'comment'
  | 'error'
  | 'note'

export interface Segment {
  text: string
  tone: Tone
}

export type Line = Segment[]

export function lineText(line: Line): string {
  return line.map((segment) => segment.text).join('')
}

/** Splits text into lines of one tone each. */
export function plainLines(text: string, tone: Tone = 'plain'): Line[] {
  return text.split('\n').map((part) => [{ text: part, tone }])
}

/** Splits toned pieces that may contain newlines into lines. */
export function toLines(pieces: Segment[]): Line[] {
  const lines: Line[] = [[]]
  for (const piece of pieces) {
    const parts = piece.text.split('\n')
    parts.forEach((part, index) => {
      if (index > 0) lines.push([])
      if (part !== '') lines.at(-1)?.push({ text: part, tone: piece.tone })
    })
  }
  return lines
}

const KEYWORDS = new Set([
  'async',
  'await',
  'break',
  'case',
  'catch',
  'class',
  'const',
  'continue',
  'default',
  'delete',
  'do',
  'else',
  'export',
  'extends',
  'false',
  'finally',
  'for',
  'function',
  'if',
  'import',
  'in',
  'instanceof',
  'let',
  'new',
  'null',
  'of',
  'return',
  'switch',
  'this',
  'throw',
  'true',
  'try',
  'typeof',
  'undefined',
  'var',
  'void',
  'while',
  'yield',
])

// Comments, the three string forms (a template literal is one string, `${}` included), words,
// and any other single character. Unterminated strings and comments run to the end.
const CODE_TOKEN =
  /\/\/[^\n]*|\/\*[\s\S]*?(?:\*\/|$)|'(?:[^'\\\n]|\\.)*'?|"(?:[^"\\\n]|\\.)*"?|`(?:[^`\\]|\\[\s\S])*`?|[A-Za-z_$][\w$]*|[\s\S]/g

/** The script as sent, line by line, with keywords, strings and comments toned. */
export function codeLines(code: string): Line[] {
  const pieces: Segment[] = []
  for (const [token] of code.matchAll(CODE_TOKEN)) {
    const tone: Tone =
      token.startsWith('//') || token.startsWith('/*')
        ? 'comment'
        : /^['"`]/.test(token)
          ? 'string'
          : KEYWORDS.has(token)
            ? 'keyword'
            : 'plain'
    const last = pieces.at(-1)
    if (last && last.tone === tone) last.text += token
    else pieces.push({ text: token, tone })
  }
  return toLines(pieces)
}

const SNAPSHOT_TOKEN = /\[ref=[^\]\s]+\]|"(?:[^"\\]|\\.)*"/g

/** An aria snapshot or other reply text: `[ref=e5]` and quoted names toned, as in the design. */
export function snapshotLine(text: string): Line {
  const line: Line = []
  let at = 0
  for (const match of text.matchAll(SNAPSHOT_TOKEN)) {
    const start = match.index ?? 0
    if (start > at) line.push({ text: text.slice(at, start), tone: 'plain' })
    line.push({
      text: match[0],
      tone: match[0].startsWith('[') ? 'ref' : 'string',
    })
    at = start + match[0].length
  }
  if (at < text.length) line.push({ text: text.slice(at), tone: 'plain' })
  return line
}

const JSON_TOKEN = /"(?:[^"\\]|\\.)*"(\s*:)?/g

/** One line of pretty-printed JSON: keys muted, strings toned. */
export function jsonLine(text: string): Line {
  const line: Line = []
  let at = 0
  for (const match of text.matchAll(JSON_TOKEN)) {
    const start = match.index ?? 0
    if (start > at) line.push({ text: text.slice(at, start), tone: 'plain' })
    line.push({ text: match[0], tone: match[1] ? 'key' : 'string' })
    at = start + match[0].length
  }
  if (at < text.length) line.push({ text: text.slice(at), tone: 'plain' })
  return line
}
