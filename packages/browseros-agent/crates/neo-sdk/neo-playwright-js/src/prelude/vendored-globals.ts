/**
 * The browser and Node globals Playwright's value serializers name, which QuickJS lacks.
 *
 * The prelude runs the vendored `isomorphic/utilityScriptSerializers.ts` and
 * `protocol/src/serializers.ts` unmodified, so the build (scripts/build.ts) loads both with these
 * four as module-scope imports, in the prelude's bundle only: the page's copies run on the
 * page's own globals. No global is added to the agent's realm (tests/prelude-globals.test.ts).
 *
 * - `URL`: a URL value in Playwright's format (`{u}`) becomes `urlLike`'s object, which has a
 *   URL's fields, so a URL result is URL-like but not `instanceof URL` (a listed deviation).
 * - `Buffer`: only `Buffer.from(buffer, byteOffset, length)`, the view the protocol serializer
 *   makes of a typed array's bytes, which its parse reads back with the same three fields.
 * - `btoa` and `atob`: base64 over strings of byte-sized characters, for typed arrays.
 */
import { urlLike } from './url'

const BASE64 =
  'ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/'

/** A URL as the agent's realm can hold one: `urlLike`'s fields, or the href alone. */
export class URL {
  readonly href: string

  constructor(href: string) {
    this.href = String(href)
    Object.assign(this, urlLike(this.href))
  }

  toString(): string {
    return this.href
  }

  toJSON(): string {
    return this.href
  }
}

/** `Buffer.from(arrayBuffer, byteOffset, length)`: a byte view, no copy, as Node's is. */
export const Buffer = {
  from: (buffer: ArrayBufferLike, byteOffset?: number, length?: number) =>
    new Uint8Array(buffer, byteOffset, length),
}

/** Base64 of `binary`, whose characters are bytes (0 to 255), as the browser's `btoa`. */
export function btoa(binary: string): string {
  let out = ''
  for (let index = 0; index < binary.length; index += 3) {
    const [a = 0, b = 0, c = 0] = [0, 1, 2].map(
      (offset) => binary.charCodeAt(index + offset) || 0,
    )
    const bits = (a << 16) | (b << 8) | c
    const left = binary.length - index
    out += BASE64.charAt((bits >> 18) & 63) + BASE64.charAt((bits >> 12) & 63)
    out += left > 1 ? BASE64.charAt((bits >> 6) & 63) : '='
    out += left > 2 ? BASE64.charAt(bits & 63) : '='
  }
  return out
}

/** The bytes of `base64` as a string of byte-sized characters, as the browser's `atob`. */
export function atob(base64: string): string {
  const digits = String(base64).replace(/[^A-Za-z0-9+/]/g, '')
  let out = ''
  let bits = 0
  let held = 0
  for (const digit of digits) {
    bits = ((bits << 6) | BASE64.indexOf(digit)) & 0xffffff
    held += 6
    if (held >= 8) {
      held -= 8
      out += String.fromCharCode((bits >> held) & 255)
    }
  }
  return out
}
