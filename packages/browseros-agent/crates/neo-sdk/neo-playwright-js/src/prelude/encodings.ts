/**
 * The text encodings Node's Buffer reads and writes, as pure functions from a string to its
 * bytes and back (buffer.ts gives them Node's API). Each is Node's: utf8, hex, base64,
 * base64url, latin1 (binary), ascii and utf16le (ucs2), named in any case, with Node's
 * handling of input an encoding cannot hold (U+FFFD for bad UTF-8, base64 that skips stray
 * characters and stops at its first `=`, hex that stops at its first non-hex pair).
 */

export type Encoding =
  | 'utf8'
  | 'hex'
  | 'base64'
  | 'base64url'
  | 'latin1'
  | 'ascii'
  | 'utf16le'

/**
 * Node's names for each encoding (`normalizeEncoding`), lowercased. A Map, so these twelve are
 * the only names: an object literal would also answer for the names it inherits, such as
 * `constructor` and `__proto__`.
 */
const ENCODINGS = new Map<string, Encoding>([
  ['utf8', 'utf8'],
  ['utf-8', 'utf8'],
  ['hex', 'hex'],
  ['base64', 'base64'],
  ['base64url', 'base64url'],
  ['latin1', 'latin1'],
  ['binary', 'latin1'],
  ['ascii', 'ascii'],
  ['utf16le', 'utf16le'],
  ['utf-16le', 'utf16le'],
  ['ucs2', 'utf16le'],
  ['ucs-2', 'utf16le'],
])

const BASE64 =
  'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'
const BASE64URL =
  'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_'

/** Each alphabet's characters as code units, which the encoder writes. */
const BASE64_UNITS = Uint16Array.from(BASE64, (char) => char.charCodeAt(0))
const BASE64URL_UNITS = Uint16Array.from(BASE64URL, (char) =>
  char.charCodeAt(0),
)

/** Each character's six bits, from either alphabet; -1 for any other character. */
const SEXTETS = (() => {
  const table = new Int16Array(128).fill(-1)
  for (let index = 0; index < 64; index++) {
    table[BASE64.charCodeAt(index)] = index
    table[BASE64URL.charCodeAt(index)] = index
  }
  return table
})()

/** Code units per `String.fromCharCode` call, well inside QuickJS's argument limit. */
const CHUNK = 0x2000

const HEX_DIGITS = '0123456789abcdef'

/** The encoding Node knows by `name`, in any case, or undefined. */
export function encodingNamed(name: string): Encoding | undefined {
  return ENCODINGS.get(name.toLowerCase())
}

/**
 * Text written one UTF-16 code unit at a time, never held whole as numbers. A run's QuickJS
 * heap is 64 MiB (claw-sdk's config/defaults.rs) and an array spends a 16-byte value on each
 * number it holds, so text kept as an array of character codes costs 16 bytes a character: 2 MiB
 * of bytes as base64 would not fit. Here a CHUNK-long typed array takes the units, each full one
 * becomes a string, and the strings join once, so text costs about twice its own size while it
 * is built.
 */
class TextWriter {
  readonly #units = new Uint16Array(CHUNK)
  readonly #parts: string[] = []
  #filled = 0

  /** Appends one UTF-16 code unit. */
  unit(code: number): void {
    this.#units[this.#filled++] = code
    if (this.#filled === CHUNK) this.#flush()
  }

  /** Appends `units` as they are, after what was written before. */
  units(units: Uint16Array): void {
    this.#flush()
    if (units.length > 0) this.#parts.push(fromUnits(units))
  }

  /** Everything written. */
  text(): string {
    this.#flush()
    return this.#parts.join('')
  }

  #flush(): void {
    if (this.#filled === 0) return
    this.#parts.push(fromUnits(this.#units.subarray(0, this.#filled)))
    this.#filled = 0
  }
}

/** At most CHUNK code units as a string. */
function fromUnits(units: Uint16Array): string {
  return String(Reflect.apply(String.fromCharCode, null, units))
}

/** UTF-8's length for `text`, a lone surrogate counted as U+FFFD's three bytes. */
function utf8Length(text: string): number {
  let length = 0
  for (let index = 0; index < text.length; index++) {
    const code = text.charCodeAt(index)
    if (code < 0x80) length += 1
    else if (code < 0x800) length += 2
    else if (
      code >= 0xd800 &&
      code <= 0xdbff &&
      isLowSurrogate(text, index + 1)
    ) {
      length += 4
      index++
    } else length += 3
  }
  return length
}

function isLowSurrogate(text: string, index: number): boolean {
  const code = text.charCodeAt(index)
  return code >= 0xdc00 && code <= 0xdfff
}

/**
 * UTF-8 of `text`, each lone surrogate as U+FFFD, as Node's (and WHATWG's) encoder. It measures
 * first and writes into bytes of that length, so the bytes are the only copy it makes.
 */
function encodeUtf8(text: string): Uint8Array {
  const out = new Uint8Array(utf8Length(text))
  let at = 0
  for (let index = 0; index < text.length; index++) {
    let point = text.charCodeAt(index)
    if (point >= 0xd800 && point <= 0xdbff) {
      if (isLowSurrogate(text, index + 1)) {
        point =
          0x10000 +
          ((point - 0xd800) << 10) +
          (text.charCodeAt(index + 1) - 0xdc00)
        index++
      } else point = 0xfffd
    } else if (point >= 0xdc00 && point <= 0xdfff) point = 0xfffd
    if (point < 0x80) out[at++] = point
    else if (point < 0x800) {
      out[at++] = 0xc0 | (point >> 6)
      out[at++] = 0x80 | (point & 63)
    } else if (point < 0x10000) {
      out[at++] = 0xe0 | (point >> 12)
      out[at++] = 0x80 | ((point >> 6) & 63)
      out[at++] = 0x80 | (point & 63)
    } else {
      out[at++] = 0xf0 | (point >> 18)
      out[at++] = 0x80 | ((point >> 12) & 63)
      out[at++] = 0x80 | ((point >> 6) & 63)
      out[at++] = 0x80 | (point & 63)
    }
  }
  return out
}

/**
 * What a UTF-8 lead byte past ASCII starts, as WHATWG's decoder reads it: how many
 * continuation bytes follow, the lead's bits, and the range the first continuation byte must
 * fall in (narrower after E0, ED, F0 and F4, which rules out overlong forms, surrogates and code
 * points past U+10FFFF). `null` for a byte that cannot lead.
 */
function leadOf(
  byte: number,
): { needed: number; point: number; lower: number; upper: number } | null {
  if (byte >= 0xc2 && byte <= 0xdf)
    return { needed: 1, point: byte & 0x1f, lower: 0x80, upper: 0xbf }
  if (byte >= 0xe0 && byte <= 0xef)
    return {
      needed: 2,
      point: byte & 0x0f,
      lower: byte === 0xe0 ? 0xa0 : 0x80,
      upper: byte === 0xed ? 0x9f : 0xbf,
    }
  if (byte >= 0xf0 && byte <= 0xf4)
    return {
      needed: 3,
      point: byte & 0x07,
      lower: byte === 0xf0 ? 0x90 : 0x80,
      upper: byte === 0xf4 ? 0x8f : 0xbf,
    }
  return null
}

/**
 * The code point of the sequence the non-ASCII byte at `index` leads, and where the next one
 * starts. A sequence that cannot continue is U+FFFD for its maximal subpart, and the next one
 * starts at the byte that broke it.
 */
function sequenceAt(bytes: Uint8Array, index: number): [number, number] {
  const lead = leadOf(bytes[index] ?? 0)
  if (lead === null) return [0xfffd, index + 1]
  let { point, lower, upper } = lead
  let next = index + 1
  for (let seen = 0; seen < lead.needed; seen++) {
    const byte = bytes[next]
    if (byte === undefined || byte < lower || byte > upper)
      return [0xfffd, next]
    point = (point << 6) | (byte & 0x3f)
    lower = 0x80
    upper = 0xbf
    next++
  }
  return [point, next]
}

/** WHATWG's UTF-8 decoder, as Node's, with U+FFFD where a sequence breaks. */
function decodeUtf8(bytes: Uint8Array): string {
  const text = new TextWriter()
  let index = 0
  while (index < bytes.length) {
    const byte = bytes[index] ?? 0
    if (byte < 0x80) {
      text.unit(byte)
      index++
      continue
    }
    const [point, next] = sequenceAt(bytes, index)
    index = next
    if (point < 0x10000) text.unit(point)
    else {
      text.unit(0xd800 + ((point - 0x10000) >> 10))
      text.unit(0xdc00 + ((point - 0x10000) & 0x3ff))
    }
  }
  return text.text()
}

/**
 * Node's base64 and base64url decoder (they accept each other's alphabet): characters of
 * neither alphabet are skipped, the first `=` ends the data, and a last lone character is
 * dropped.
 */
function decodeBase64(text: string): Uint8Array {
  const out = new Uint8Array(Math.floor((text.length * 3) / 4))
  let length = 0
  let bits = 0
  let held = 0
  for (let index = 0; index < text.length; index++) {
    const code = text.charCodeAt(index)
    if (code === 0x3d) break
    const sextet = code < 128 ? (SEXTETS[code] ?? -1) : -1
    if (sextet < 0) continue
    held = ((held << 6) | sextet) & 0xffffff
    bits += 6
    if (bits >= 8) {
      bits -= 8
      out[length++] = (held >> bits) & 0xff
    }
  }
  return out.subarray(0, length)
}

/**
 * Node's base64 and base64url encoder, the text a payload's bytes cross to the page as. Whole
 * groups go into a CHUNK-long block rather than one `unit` call each: it is the hot path of a
 * large `setInputFiles` payload, and twice as fast this way in QuickJS.
 */
function encodeBase64(
  bytes: Uint8Array,
  alphabet: Uint16Array,
  pad: boolean,
): string {
  const text = new TextWriter()
  const block = new Uint16Array(CHUNK)
  let filled = 0
  let index = 0
  for (; index + 2 < bytes.length; index += 3) {
    const n =
      ((bytes[index] ?? 0) << 16) |
      ((bytes[index + 1] ?? 0) << 8) |
      (bytes[index + 2] ?? 0)
    block[filled] = alphabet[(n >> 18) & 63] ?? 0
    block[filled + 1] = alphabet[(n >> 12) & 63] ?? 0
    block[filled + 2] = alphabet[(n >> 6) & 63] ?? 0
    block[filled + 3] = alphabet[n & 63] ?? 0
    filled += 4
    if (filled === CHUNK) {
      text.units(block)
      filled = 0
    }
  }
  text.units(block.subarray(0, filled))
  const left = bytes.length - index
  if (left > 0) {
    const n = ((bytes[index] ?? 0) << 16) | ((bytes[index + 1] ?? 0) << 8)
    text.unit(alphabet[(n >> 18) & 63] ?? 0)
    text.unit(alphabet[(n >> 12) & 63] ?? 0)
    if (left === 2) text.unit(alphabet[(n >> 6) & 63] ?? 0)
    else if (pad) text.unit(0x3d)
    if (pad) text.unit(0x3d)
  }
  return text.text()
}

/** Node's hex decoder: pairs of hex digits up to the first pair that is not one. */
function decodeHex(text: string): Uint8Array {
  const out = new Uint8Array(text.length >> 1)
  let length = 0
  for (let index = 0; index + 1 < text.length; index += 2) {
    const pair = text.slice(index, index + 2)
    if (!/^[0-9a-fA-F]{2}$/.test(pair)) break
    out[length++] = Number.parseInt(pair, 16)
  }
  return out.subarray(0, length)
}

/** `text`'s bytes in `encoding`, as Node's `Buffer.from(text, encoding)`. */
export function encode(text: string, encoding: Encoding): Uint8Array {
  switch (encoding) {
    case 'utf8':
      return encodeUtf8(text)
    case 'hex':
      return decodeHex(text)
    case 'base64':
    case 'base64url':
      return decodeBase64(text)
    case 'latin1':
    case 'ascii': {
      const out = new Uint8Array(text.length)
      for (let index = 0; index < text.length; index++)
        out[index] = text.charCodeAt(index) & 0xff
      return out
    }
    case 'utf16le': {
      const out = new Uint8Array(text.length * 2)
      for (let index = 0; index < text.length; index++) {
        const code = text.charCodeAt(index)
        out[index * 2] = code & 0xff
        out[index * 2 + 1] = code >> 8
      }
      return out
    }
  }
}

/** `bytes` as text in `encoding`, as Node's `buffer.toString(encoding)`. */
export function decode(bytes: Uint8Array, encoding: Encoding): string {
  switch (encoding) {
    case 'utf8':
      return decodeUtf8(bytes)
    case 'hex': {
      const text = new TextWriter()
      for (const byte of bytes) {
        text.unit(HEX_DIGITS.charCodeAt(byte >> 4))
        text.unit(HEX_DIGITS.charCodeAt(byte & 15))
      }
      return text.text()
    }
    case 'base64':
      return encodeBase64(bytes, BASE64_UNITS, true)
    case 'base64url':
      return encodeBase64(bytes, BASE64URL_UNITS, false)
    case 'latin1':
    case 'ascii': {
      const mask = encoding === 'ascii' ? 0x7f : 0xff
      const text = new TextWriter()
      for (const byte of bytes) text.unit(byte & mask)
      return text.text()
    }
    case 'utf16le': {
      const text = new TextWriter()
      for (let index = 0; index + 1 < bytes.length; index += 2)
        text.unit((bytes[index] ?? 0) | ((bytes[index + 1] ?? 0) << 8))
      return text.text()
    }
  }
}
