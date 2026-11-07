/**
 * Neo: the realm's bytes for Playwright's protocol `binary`, which crosses the wire as base64
 * text in both directions (neo-runtime-spec `Binary` on the Rust side). Every result that
 * carries bytes resolves a `Buffer` (`page.screenshot`, `locator.screenshot`, `page.pdf`,
 * `response.body`), and every input that carries bytes sends `toBinary(bytes)`.
 *
 * ```text
 *  Rust Binary(Vec<u8>) ── "iVBORw0KGgo…" ──▶ Buffer.fromBinary ──▶ Buffer (a Uint8Array)
 *  Rust Binary(Vec<u8>) ◀── "aGVsbG8="     ◀── toBinary(Uint8Array | Buffer)
 * ```
 *
 * `Buffer` is the part of Node's `Buffer` a script reads results with, moved unchanged in
 * behaviour from `network.ts` (#3172). QuickJS has no `Buffer`, and the SDK defines no globals.
 *
 * Platform: this QuickJS (quickjs-ng in rquickjs 0.12) has `Uint8Array.fromBase64` and
 * `Uint8Array.prototype.toBase64` (probes-bin.md B3: a 10 MiB result decodes in 6 ms and
 * encodes in 2 ms, where a JavaScript loop took 1.5 s and 3.7 s), so both directions use them.
 */

/** The two base64 natives, which TypeScript's ES2023 library does not declare. */
type Base64Natives = {
  fromBase64(
    text: string,
    options?: { alphabet?: 'base64' | 'base64url' },
  ): Uint8Array
}
type Base64Bytes = Uint8Array & {
  toBase64(options?: {
    alphabet?: 'base64' | 'base64url'
    omitPadding?: boolean
  }): string
}

/** The wire's `binary` for `bytes`: standard base64 with padding, which Rust decodes. */
export function toBinary(bytes: Uint8Array): string {
  return (bytes as Base64Bytes).toBase64()
}

/**
 * The part of Node's `Buffer` a body needs: a `Uint8Array` whose `toString` decodes as Node's
 * does (UTF-8 by default), whose JSON is Node's `{ type: 'Buffer', data }`, and `equals`.
 */
export class Buffer extends Uint8Array {
  /** The bytes of the wire's `binary`, a base64 string, without a copy. Rust always sends
   * standard base64 with padding; anything else is a broken bridge. */
  static fromBinary(text: string): Buffer {
    let bytes: Uint8Array
    try {
      bytes = (Uint8Array as unknown as Base64Natives).fromBase64(text)
    } catch {
      throw new Error('neo-sdk: invalid base64 from Rust')
    }
    return new Buffer(
      bytes.buffer as ArrayBuffer,
      bytes.byteOffset,
      bytes.length,
    )
  }

  /**
   * Node's `Buffer.prototype.toString` (`lib/buffer.js`): with no argument the whole buffer as
   * UTF-8. Otherwise the range is clamped and truncated first, so an empty range is `''` whatever
   * the encoding; then the encoding is read case-insensitively, an unknown one a `TypeError`.
   */
  override toString(
    ...args: [encoding?: string | null, start?: number, end?: number]
  ): string {
    if (args.length === 0) return decodeUtf8(this)
    const [encoding, start, end] = args
    const length = this.length
    let from: number
    if ((start as number) <= 0) from = 0
    else if ((start as number) >= length) return ''
    else from = Math.trunc(start as number) || 0
    const to = end === undefined || end > length ? length : Math.trunc(end) || 0
    if (to <= from) return ''
    const bytes = this.subarray(from, to)
    if (encoding === undefined) return decodeUtf8(bytes)
    switch (String(encoding).toLowerCase()) {
      case 'utf8':
      case 'utf-8':
        return decodeUtf8(bytes)
      case 'ucs2':
      case 'ucs-2':
      case 'utf16le':
      case 'utf-16le':
        return fromCodeUnits(utf16Units(bytes), (unit) => unit)
      case 'latin1':
      case 'binary':
        return fromCodeUnits(bytes, (byte) => byte)
      case 'ascii':
        return fromCodeUnits(bytes, (byte) => byte & 0x7f)
      case 'base64':
        return toBinary(bytes)
      case 'base64url':
        return (bytes as Base64Bytes).toBase64({
          alphabet: 'base64url',
          omitPadding: true,
        })
      case 'hex':
        return [...bytes]
          .map((byte) => byte.toString(16).padStart(2, '0'))
          .join('')
      default:
        throw new TypeError(`Unknown encoding: ${encoding}`)
    }
  }

  toJSON(): { type: 'Buffer'; data: number[] } {
    return { type: 'Buffer', data: [...this] }
  }

  equals(other: Uint8Array): boolean {
    return (
      this.length === other.length && this.every((byte, i) => byte === other[i])
    )
  }
}

/** A UTF-8 lead byte's sequence: continuation bytes still needed, its payload bits, and the
 * range its first continuation byte must fall in (the WHATWG decoder's bounds). */
type Lead = { needed: number; codePoint: number; lower: number; upper: number }

function utf8Lead(byte: number): Lead | null {
  if (byte >= 0xc2 && byte <= 0xdf)
    return { needed: 1, codePoint: byte & 0x1f, lower: 0x80, upper: 0xbf }
  if (byte >= 0xe0 && byte <= 0xef)
    return {
      needed: 2,
      codePoint: byte & 0xf,
      lower: byte === 0xe0 ? 0xa0 : 0x80,
      upper: byte === 0xed ? 0x9f : 0xbf,
    }
  if (byte >= 0xf0 && byte <= 0xf4)
    return {
      needed: 3,
      codePoint: byte & 0x7,
      lower: byte === 0xf0 ? 0x90 : 0x80,
      upper: byte === 0xf4 ? 0x8f : 0xbf,
    }
  return null
}

/**
 * The WHATWG UTF-8 decoder, which Node's `buffer.toString('utf8')` follows: each maximal
 * invalid subsequence becomes one U+FFFD. QuickJS has no `TextDecoder`.
 */
export function decodeUtf8(bytes: Uint8Array): string {
  const units = new CodeUnits()
  let lead: Lead | null = null
  for (let i = 0; i < bytes.length; i++) {
    const byte = bytes[i] as number
    if (!lead) {
      lead = byte <= 0x7f ? null : utf8Lead(byte)
      if (byte <= 0x7f) units.push(byte)
      else if (!lead) units.push(0xfffd)
      continue
    }
    if (byte < lead.lower || byte > lead.upper) {
      // The sequence ends here; this byte starts over.
      lead = null
      units.push(0xfffd)
      i--
      continue
    }
    lead.codePoint = (lead.codePoint << 6) | (byte & 0x3f)
    lead.lower = 0x80
    lead.upper = 0xbf
    lead.needed--
    if (lead.needed === 0) {
      units.push(lead.codePoint)
      lead = null
    }
  }
  if (lead) units.push(0xfffd)
  return units.text()
}

/** UTF-16 code units collected in chunks, so a long body never spreads one huge array. */
class CodeUnits {
  #units: number[] = []
  #out = ''

  push(codePoint: number): void {
    if (codePoint > 0xffff) {
      const offset = codePoint - 0x10000
      this.#units.push(0xd800 + (offset >> 10), 0xdc00 + (offset & 0x3ff))
    } else {
      this.#units.push(codePoint)
    }
    if (this.#units.length >= 4096) this.#flush()
  }

  text(): string {
    this.#flush()
    return this.#out
  }

  #flush(): void {
    this.#out += String.fromCharCode(...this.#units)
    this.#units = []
  }
}

/** Little-endian 16-bit code units; an odd last byte is dropped, as Node drops it. */
function utf16Units(bytes: Uint8Array): Uint16Array {
  const units = new Uint16Array(bytes.length >> 1)
  for (let i = 0; i < units.length; i++)
    units[i] = (bytes[2 * i] as number) | ((bytes[2 * i + 1] as number) << 8)
  return units
}

function fromCodeUnits(
  bytes: Uint8Array | Uint16Array,
  unit: (byte: number) => number,
): string {
  let out = ''
  for (let i = 0; i < bytes.length; i += 4096)
    out += String.fromCharCode(...[...bytes.subarray(i, i + 4096)].map(unit))
  return out
}
