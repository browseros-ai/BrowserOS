/**
 * Positions of JSON values in the text as sent. The proxy must forward every byte the agent
 * sent except the one field it adds, so it never re-serializes a request: it finds where a value
 * sits and splices text into it. `JSON.parse` + `JSON.stringify` would change whitespace,
 * escapes and big numbers.
 *
 * The scanner trusts its input to be valid JSON (callers check with `JSON.parse` first) and
 * returns `null` when it is not.
 */

export interface Span {
  /** Index of the value's first character. */
  start: number
  /** Index just past the value's last character. */
  end: number
}

export interface Member {
  key: string
  value: Span
}

const WHITESPACE = new Set([' ', '\t', '\n', '\r'])

export function skipWhitespace(text: string, index: number): number {
  let i = index
  while (i < text.length && WHITESPACE.has(text[i] as string)) i++
  return i
}

function stringEnd(text: string, start: number): number | null {
  let i = start + 1
  while (i < text.length) {
    const char = text[i]
    if (char === '\\') i += 2
    else if (char === '"') return i + 1
    else i++
  }
  return null
}

function containerEnd(text: string, start: number): number | null {
  const close = text[start] === '{' ? '}' : ']'
  let i = skipWhitespace(text, start + 1)
  if (text[i] === close) return i + 1
  while (i < text.length) {
    if (close === '}') {
      const keyEnd = text[i] === '"' ? stringEnd(text, i) : null
      if (keyEnd === null) return null
      i = skipWhitespace(text, keyEnd)
      if (text[i] !== ':') return null
      i = skipWhitespace(text, i + 1)
    }
    const end = valueEnd(text, i)
    if (end === null) return null
    i = skipWhitespace(text, end)
    if (text[i] === close) return i + 1
    if (text[i] !== ',') return null
    i = skipWhitespace(text, i + 1)
  }
  return null
}

/** Index just past the JSON value starting at `start` (no leading whitespace). */
function valueEnd(text: string, start: number): number | null {
  const char = text[start]
  if (char === '"') return stringEnd(text, start)
  if (char === '{' || char === '[') return containerEnd(text, start)
  let i = start
  while (i < text.length && /[-+.\w]/.test(text[i] as string)) i++
  return i > start ? i : null
}

/** The members of the object at `start` (which must be `{`), in order, with decoded keys. */
export function objectMembers(text: string, start: number): Member[] | null {
  if (text[start] !== '{') return null
  const members: Member[] = []
  let i = skipWhitespace(text, start + 1)
  if (text[i] === '}') return members
  while (i < text.length) {
    const keyEnd = text[i] === '"' ? stringEnd(text, i) : null
    if (keyEnd === null) return null
    const key = JSON.parse(text.slice(i, keyEnd)) as string
    i = skipWhitespace(text, keyEnd)
    if (text[i] !== ':') return null
    const valueStart = skipWhitespace(text, i + 1)
    const end = valueEnd(text, valueStart)
    if (end === null) return null
    members.push({ key, value: { start: valueStart, end } })
    i = skipWhitespace(text, end)
    if (text[i] === '}') return members
    if (text[i] !== ',') return null
    i = skipWhitespace(text, i + 1)
  }
  return null
}

/** The items of the array at `start` (which must be `[`), in order. */
export function arrayItems(text: string, start: number): Span[] | null {
  if (text[start] !== '[') return null
  const items: Span[] = []
  let i = skipWhitespace(text, start + 1)
  if (text[i] === ']') return items
  while (i < text.length) {
    const end = valueEnd(text, i)
    if (end === null) return null
    items.push({ start: i, end })
    i = skipWhitespace(text, end)
    if (text[i] === ']') return items
    if (text[i] !== ',') return null
    i = skipWhitespace(text, i + 1)
  }
  return null
}

/**
 * The last member named `key`, as `JSON.parse` (and serde_json on Neo's side, for maps) would
 * read a duplicated key.
 */
export function findMember(members: Member[], key: string): Member | undefined {
  for (let i = members.length - 1; i >= 0; i--) {
    if (members[i]?.key === key) return members[i]
  }
  return undefined
}
