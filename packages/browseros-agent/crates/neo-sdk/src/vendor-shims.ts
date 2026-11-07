/**
 * Neo: module-scoped stand-ins for the four globals Playwright's vendored value serializers use
 * and QuickJS lacks: `URL`, `Buffer`, `atob` and `btoa`. `build.ts` binds them inside those two
 * modules only (`vendor/playwright/protocol/src/serializers.ts` and
 * `vendor/playwright/isomorphic/utilityScriptSerializers.ts`), by an import it prepends while
 * bundling. Nothing here is a global, and the vendored files stay byte-identical.
 *
 * Each is the least that the serializers' own calls need: `new URL(href)` / `instanceof URL`,
 * `Buffer.from(arrayBuffer, byteOffset, length)`, and binary-string base64.
 */

/** Neo: base64's 64 digits (RFC 4648 section 4), for the serializers' typed-array transport. */
const DIGITS =
  'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

/**
 * Deviation: `isomorphic/utilityScriptSerializers.ts` `parseEvaluationResultValue`: `return new URL(value.u);`
 * QuickJS has no WHATWG URL, so a `URL` the page returns (and a `URL` an agent passes back) is
 * this object: its `href`, `toString()` and `toJSON()` as Playwright's client gives them, and no
 * parsed parts (`host`, `pathname`, `searchParams`).
 */
export class URL {
  readonly href: string

  constructor(href: string) {
    this.href = String(href)
  }

  toString(): string {
    return this.href
  }

  toJSON(): string {
    return this.href
  }

  get [Symbol.toStringTag](): string {
    return 'URL'
  }
}

/** Neo: the one `Buffer` call of `protocol/src/serializers.ts`, a view over a typed array's
 * bytes, which `parseSerializedValue` reads straight back in the same realm. */
export const Buffer = {
  from(
    buffer: ArrayBufferLike,
    byteOffset: number,
    length: number,
  ): Uint8Array {
    return new Uint8Array(buffer, byteOffset, length)
  },
}

/** Neo: `btoa` over a binary string (each char code one byte), for typed arrays sent to the page. */
export function btoa(binary: string): string {
  let out = ''
  for (let i = 0; i < binary.length; i += 3) {
    const a = binary.charCodeAt(i)
    const b = i + 1 < binary.length ? binary.charCodeAt(i + 1) : -1
    const c = i + 2 < binary.length ? binary.charCodeAt(i + 2) : -1
    if (a > 0xff || b > 0xff || c > 0xff)
      throw new Error('btoa: the string has characters outside Latin1')
    out += DIGITS[a >> 2]
    out += DIGITS[((a & 3) << 4) | (b < 0 ? 0 : b >> 4)]
    out += b < 0 ? '=' : DIGITS[((b & 15) << 2) | (c < 0 ? 0 : c >> 6)]
    out += c < 0 ? '=' : DIGITS[c & 63]
  }
  return out
}

/** Neo: `atob` to a binary string, for typed arrays the page returns. */
export function atob(base64: string): string {
  const clean = base64.replace(/[\t\n\f\r ]/g, '').replace(/={1,2}$/, '')
  let out = ''
  let bits = 0
  let value = 0
  for (const char of clean) {
    const digit = DIGITS.indexOf(char)
    if (digit < 0) throw new Error('atob: the string is not valid base64')
    value = (value << 6) | digit
    bits += 6
    if (bits >= 8) {
      bits -= 8
      out += String.fromCharCode((value >> bits) & 0xff)
    }
  }
  return out
}
