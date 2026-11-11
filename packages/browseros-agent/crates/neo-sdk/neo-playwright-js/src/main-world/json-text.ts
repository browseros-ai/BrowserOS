/**
 * The JSON text the main-world function and Rust exchange, read and written without the page's
 * globals. A page owns `JSON`, `Array.prototype.toJSON` and the array methods, and may replace
 * them (Playwright's own specs set `window.JSON = null`, add an array `toJSON`, null out
 * `Array.prototype.map`). Upstream never meets them on this path: CDP carries its arguments and
 * results by value. Neo carries them as text (index.ts says why), so the text is spelled here
 * with string primitives and operators only, plus `Object.keys`, `Array.isArray` and
 * `String.fromCharCode`, which upstream's serializers lean on as well.
 *
 * The values are Playwright's serialized format (`SerializedValue`): plain objects, arrays,
 * strings, finite numbers, booleans and null. `writeJson` spells them as `JSON.stringify` does,
 * byte for byte, since Rust's strict parse reads that text (`page/page_value.rs`), and drops what
 * `JSON.stringify` drops; `readJson` reads the text the prelude's `JSON.stringify` wrote.
 */

const HEX = '0123456789abcdef'

/** `JSON.stringify`'s escape for a control character: a short one where it has one. */
function controlEscape(code: number): string {
  switch (code) {
    case 0x08:
      return '\\b'
    case 0x09:
      return '\\t'
    case 0x0a:
      return '\\n'
    case 0x0c:
      return '\\f'
    case 0x0d:
      return '\\r'
    default:
      return unicodeEscape(code)
  }
}

const unicodeEscape = (code: number): string =>
  `\\u${HEX[(code >> 12) & 15]}${HEX[(code >> 8) & 15]}${HEX[(code >> 4) & 15]}${HEX[code & 15]}`

/**
 * `JSON.stringify`'s string quoting (ECMA-262 QuoteJSONString): `"` and `\`, the control
 * characters, and a lone surrogate are escaped; everything else is copied in runs.
 */
function quote(text: string): string {
  let out = '"'
  let start = 0
  for (let at = 0; at < text.length; at++) {
    const code = text.charCodeAt(at)
    let escaped: string
    if (code === 0x22) escaped = '\\"'
    else if (code === 0x5c) escaped = '\\\\'
    else if (code < 0x20) escaped = controlEscape(code)
    else if (code >= 0xd800 && code <= 0xdfff) {
      const next = text.charCodeAt(at + 1)
      if (code <= 0xdbff && next >= 0xdc00 && next <= 0xdfff) {
        at++
        continue
      }
      escaped = unicodeEscape(code)
    } else continue
    out += text.slice(start, at) + escaped
    start = at + 1
  }
  return `${out}${text.slice(start)}"`
}

/** One value's text, or `undefined` for what `JSON.stringify` leaves out. */
export function writeJson(value: unknown): string | undefined {
  if (value === null) return 'null'
  switch (typeof value) {
    case 'string':
      return quote(value)
    case 'number':
      // A primitive's ToString, as JSON.stringify spells a finite number; NaN and the
      // infinities, whose difference with themselves is not 0, are null.
      return value - value === 0 ? `${value}` : 'null'
    case 'boolean':
      return value ? 'true' : 'false'
    case 'object':
      break
    default:
      return undefined
  }
  if (Array.isArray(value)) {
    let out = '['
    for (let index = 0; index < value.length; index++) {
      if (index > 0) out += ','
      out += writeJson(value[index]) ?? 'null'
    }
    return `${out}]`
  }
  const record = value as Record<string, unknown>
  const keys = Object.keys(record)
  let out = '{'
  let first = true
  for (let index = 0; index < keys.length; index++) {
    const key = keys[index] as string
    const text = writeJson(record[key])
    if (text === undefined) continue
    out += `${first ? '' : ','}${quote(key)}:${text}`
    first = false
  }
  return `${out}}`
}

/**
 * The value of JSON text, as `JSON.parse` reads it. The text is the prelude's own
 * `JSON.stringify` output, so a malformed one is Neo's bug and throws.
 */
export function readJson(text: string): unknown {
  const reader = new JsonReader(text)
  const value = reader.value()
  reader.end()
  return value
}

/** A recursive-descent reader of RFC 8259 JSON, one method per production. */
class JsonReader {
  private at = 0

  constructor(private readonly text: string) {}

  value(): unknown {
    this.skipSpace()
    switch (this.text[this.at]) {
      case '{':
        return this.object()
      case '[':
        return this.array()
      case '"':
        return this.string()
      case 't':
        return this.literal('true', true)
      case 'f':
        return this.literal('false', false)
      case 'n':
        return this.literal('null', null)
      default:
        return this.number()
    }
  }

  /** Only space may follow the value. */
  end(): void {
    this.skipSpace()
    if (this.at !== this.text.length) this.fail()
  }

  /**
   * An object without a prototype, so every member is an own data property as `JSON.parse`
   * makes it: an assignment on an ordinary object would run a setter the page put on
   * `Object.prototype` (or `__proto__`'s own), and lose the member.
   */
  private object(): Record<string, unknown> {
    this.take('{')
    const out = { __proto__: null } as unknown as Record<string, unknown>
    if (this.next('}')) return out
    do {
      this.skipSpace()
      const key = this.string()
      this.skipSpace()
      this.take(':')
      out[key] = this.value()
    } while (this.next(','))
    this.skipSpace()
    this.take('}')
    return out
  }

  private array(): unknown[] {
    this.take('[')
    const out: unknown[] = []
    if (this.next(']')) return out
    do out[out.length] = this.value()
    while (this.next(','))
    this.skipSpace()
    this.take(']')
    return out
  }

  private string(): string {
    const { text } = this
    this.take('"')
    let out = ''
    let start = this.at
    for (;;) {
      // A raw control character is not JSON, and neither is text that ends inside a string.
      if (this.at >= text.length || text.charCodeAt(this.at) < 0x20) this.fail()
      const char = text[this.at]
      if (char === '"') break
      if (char !== '\\') {
        this.at++
        continue
      }
      out += text.slice(start, this.at) + this.escape()
      start = this.at
    }
    out += text.slice(start, this.at)
    this.at++
    return out
  }

  /** The character of the escape at a backslash, which it reads past. */
  private escape(): string {
    const kind = this.text[this.at + 1]
    this.at += 2
    if (kind !== 'u') return unescaped(kind) ?? this.fail()
    let unit = 0
    for (const end = this.at + 4; this.at < end; this.at++)
      unit =
        unit * 16 + (hexDigit(this.text.charCodeAt(this.at)) ?? this.fail())
    return String.fromCharCode(unit)
  }

  /** `-? (0 | [1-9][0-9]*) (. [0-9]+)? ([eE] [+-]? [0-9]+)?`, read by the primitive's ToNumber. */
  private number(): number {
    const start = this.at
    this.eat('-')
    if (!this.eat('0')) this.digits()
    if (this.eat('.')) this.digits()
    if (this.eat('e') || this.eat('E')) {
      if (!this.eat('+')) this.eat('-')
      this.digits()
    }
    return +this.text.slice(start, this.at)
  }

  /** One or more decimal digits. */
  private digits(): void {
    const start = this.at
    while (isDigit(this.text.charCodeAt(this.at))) this.at++
    if (this.at === start) this.fail()
  }

  private literal<T>(word: string, value: T): T {
    if (this.text.slice(this.at, this.at + word.length) !== word) this.fail()
    this.at += word.length
    return value
  }

  /** Reads past `char` if it comes next, after any space. */
  private next(char: string): boolean {
    this.skipSpace()
    return this.eat(char)
  }

  /** Reads past `char` if it is the very next character. */
  private eat(char: string): boolean {
    if (this.text[this.at] !== char) return false
    this.at++
    return true
  }

  private take(char: string): void {
    if (this.text[this.at] !== char) this.fail()
    this.at++
  }

  private skipSpace(): void {
    for (;;) {
      const code = this.text.charCodeAt(this.at)
      if (code !== 0x20 && code !== 0x09 && code !== 0x0a && code !== 0x0d)
        return
      this.at++
    }
  }

  private fail(): never {
    throw new SyntaxError(`Neo: malformed JSON argument at ${this.at}`)
  }
}

/** The character a one-letter escape after `\` stands for, or `undefined` for no escape. */
function unescaped(kind: string | undefined): string | undefined {
  switch (kind) {
    case '"':
    case '\\':
    case '/':
      return kind
    case 'b':
      return '\b'
    case 'f':
      return '\f'
    case 'n':
      return '\n'
    case 'r':
      return '\r'
    case 't':
      return '\t'
    default:
      return undefined
  }
}

/** A hex digit's value, or `undefined` for a character that is not one. */
function hexDigit(code: number): number | undefined {
  if (isDigit(code)) return code - 0x30
  const lower = code | 0x20
  return lower >= 0x61 && lower <= 0x66 ? lower - 0x57 : undefined
}

const isDigit = (code: number): boolean => code >= 0x30 && code <= 0x39
