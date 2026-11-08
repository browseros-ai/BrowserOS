/**
 * Neo: the realm's `TextEncoder` and `TextDecoder` (WHATWG Encoding Standard), which QuickJS lacks
 * and a Node Playwright script has as globals. `realm.rs` installs both (`globals.ts`).
 *
 * UTF-8 only: Node built with full ICU also decodes legacy encodings (`windows-1252`,
 * `utf-16le`, …); here those labels throw the `RangeError` Node throws for an encoding it lacks.
 * The codec itself is `buffer.ts`'s, so `Buffer` and these agree byte for byte.
 */

import { decodeUtf8, encodeUtf8 } from './buffer'

/** Platform: the Encoding Standard's labels for UTF-8 (https://encoding.spec.whatwg.org/#names-and-labels). */
const UTF8_LABELS = [
  'unicode-1-1-utf-8',
  'unicode11utf8',
  'unicode20utf8',
  'utf-8',
  'utf8',
  'x-unicode20utf8',
]

export class TextEncoder {
  get encoding(): string {
    return 'utf-8'
  }

  encode(input = ''): Uint8Array {
    return encodeUtf8(String(input))
  }

  /** Encodes as much of `source` as fits in `destination`, never half a code point. */
  encodeInto(
    source: string,
    destination: Uint8Array,
  ): { read: number; written: number } {
    const text = String(source)
    let read = 0
    let written = 0
    while (read < text.length) {
      const units = (text.codePointAt(read) as number) > 0xffff ? 2 : 1
      const bytes = encodeUtf8(text.slice(read, read + units))
      if (written + bytes.length > destination.length) break
      destination.set(bytes, written)
      written += bytes.length
      read += units
    }
    return { read, written }
  }

  get [Symbol.toStringTag](): string {
    return 'TextEncoder'
  }
}

export class TextDecoder {
  readonly #fatal: boolean
  readonly #ignoreBOM: boolean
  /** The start of a sequence a streamed `decode` ended inside, held for the next call. */
  #held = new Uint8Array(0)
  /** Whether this stream has produced text, after which a U+FEFF is text, not a BOM. */
  #started = false

  constructor(
    label = 'utf-8',
    options: { fatal?: boolean; ignoreBOM?: boolean } = {},
  ) {
    if (!UTF8_LABELS.includes(String(label).trim().toLowerCase()))
      throw new RangeError(`The "${label}" encoding is not supported`)
    this.#fatal = Boolean(options?.fatal)
    this.#ignoreBOM = Boolean(options?.ignoreBOM)
  }

  get encoding(): string {
    return 'utf-8'
  }
  get fatal(): boolean {
    return this.#fatal
  }
  get ignoreBOM(): boolean {
    return this.#ignoreBOM
  }

  /**
   * `input`'s text. With `{ stream: true }` a sequence cut at the end waits for the next call;
   * without it the stream ends here, and a cut sequence is an error (U+FFFD, or a `TypeError`
   * when `fatal`).
   */
  decode(
    input?: ArrayBuffer | ArrayBufferView,
    options?: { stream?: boolean },
  ): string {
    const fresh = bytesOf(input)
    let bytes = fresh
    if (this.#held.length > 0) {
      bytes = new Uint8Array(this.#held.length + fresh.length)
      bytes.set(this.#held)
      bytes.set(fresh, this.#held.length)
    }
    const stream = Boolean(options?.stream)
    const cut = stream ? openTail(bytes) : 0
    const whole = bytes.subarray(0, bytes.length - cut)
    this.#held = bytes.slice(bytes.length - cut)
    if (this.#fatal && !isUtf8(whole)) {
      this.#reset()
      throw new TypeError('The encoded data was not valid for encoding utf-8')
    }
    let text = decodeUtf8(whole)
    if (!this.#started && !this.#ignoreBOM && text.startsWith('﻿'))
      text = text.slice(1)
    if (whole.length > 0) this.#started = true
    if (!stream) this.#reset()
    return text
  }

  get [Symbol.toStringTag](): string {
    return 'TextDecoder'
  }

  #reset(): void {
    this.#held = new Uint8Array(0)
    this.#started = false
  }
}

/** The bytes of a `BufferSource`, without a copy. */
function bytesOf(input: unknown): Uint8Array {
  if (input === undefined) return new Uint8Array(0)
  if (input instanceof ArrayBuffer || input instanceof SharedArrayBuffer)
    return new Uint8Array(input)
  if (ArrayBuffer.isView(input))
    return new Uint8Array(input.buffer, input.byteOffset, input.byteLength)
  throw new TypeError(
    'The "input" argument must be an instance of ArrayBuffer or ArrayBufferView.',
  )
}

/** A UTF-8 lead byte's continuation count and the bounds of its first continuation byte (the
 * Encoding Standard's UTF-8 decoder), or `undefined` for a byte no sequence starts with. */
function lead(
  byte: number,
): { needed: number; lower: number; upper: number } | undefined {
  if (byte >= 0xc2 && byte <= 0xdf)
    return { needed: 1, lower: 0x80, upper: 0xbf }
  if (byte >= 0xe0 && byte <= 0xef)
    return {
      needed: 2,
      lower: byte === 0xe0 ? 0xa0 : 0x80,
      upper: byte === 0xed ? 0x9f : 0xbf,
    }
  if (byte >= 0xf0 && byte <= 0xf4)
    return {
      needed: 3,
      lower: byte === 0xf0 ? 0x90 : 0x80,
      upper: byte === 0xf4 ? 0x8f : 0xbf,
    }
  return undefined
}

/** How many of the last bytes are a valid sequence cut short, which a stream holds back. */
function openTail(bytes: Uint8Array): number {
  for (let back = 1; back <= Math.min(3, bytes.length); back++) {
    const byte = bytes[bytes.length - back] as number
    if (byte < 0x80) return 0
    if (byte < 0xc0) continue
    const sequence = lead(byte)
    if (!sequence || back > sequence.needed) return 0
    const second = bytes[bytes.length - back + 1]
    const fits =
      second === undefined ||
      (second >= sequence.lower && second <= sequence.upper)
    return fits ? back : 0
  }
  return 0
}

/** Whether `bytes` are UTF-8 with no invalid or cut sequence. */
function isUtf8(bytes: Uint8Array): boolean {
  for (let i = 0; i < bytes.length; i++) {
    const byte = bytes[i] as number
    if (byte < 0x80) continue
    const sequence = lead(byte)
    if (!sequence) return false
    for (let k = 1; k <= sequence.needed; k++) {
      const next = bytes[i + k]
      const [lower, upper] =
        k === 1 ? [sequence.lower, sequence.upper] : [0x80, 0xbf]
      if (next === undefined || next < lower || next > upper) return false
    }
    i += sequence.needed
  }
  return true
}
