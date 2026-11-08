/**
 * Neo: the realm's `crypto`, the two Web Crypto calls a Node Playwright script uses for random
 * values: `crypto.randomUUID()` and `crypto.getRandomValues()`. QuickJS has neither, and its
 * `Math.random` is not a CSPRNG, so the bytes come from Rust (`randomValues` in
 * `runtime-bridge.ts`, an OS-seeded CSPRNG). `realm.rs` installs it (`globals.ts`).
 *
 * `crypto.subtle` is not served: Playwright scripts do not hash or sign, and it is large.
 */

import { randomValues } from './runtime-bridge'

/** Platform: Web Crypto's `getRandomValues` refuses an array of more than 65,536 bytes with a
 * `QuotaExceededError` (W3C Web Cryptography API, `Crypto.getRandomValues`). */
const MAX_RANDOM_BYTES = 65536

/** Platform: the integer typed arrays `getRandomValues` accepts (W3C Web Cryptography API). */
const INTEGER_ARRAYS = [
  Int8Array,
  Uint8Array,
  Uint8ClampedArray,
  Int16Array,
  Uint16Array,
  Int32Array,
  Uint32Array,
  BigInt64Array,
  BigUint64Array,
]

/** Fills an integer typed array with random values in place and returns it. */
function getRandomValues<T extends ArrayBufferView>(array: T): T {
  if (!INTEGER_ARRAYS.some((kind) => array instanceof kind))
    throw new DOMException(
      'The data argument must be an integer-type TypedArray',
      'TypeMismatchError',
    )
  if (array.byteLength > MAX_RANDOM_BYTES)
    throw new DOMException(
      `The ArrayBufferView's byte length (${array.byteLength}) exceeds the number of bytes of entropy available via this API (${MAX_RANDOM_BYTES})`,
      'QuotaExceededError',
    )
  new Uint8Array(array.buffer, array.byteOffset, array.byteLength).set(
    randomValues(array.byteLength),
  )
  return array
}

/** A random RFC 9562 version 4 UUID, lowercase, as Node's `crypto.randomUUID()`. */
function randomUUID(): string {
  const bytes = randomValues(16)
  bytes[6] = ((bytes[6] as number) & 0x0f) | 0x40
  bytes[8] = ((bytes[8] as number) & 0x3f) | 0x80
  const hex = [...bytes]
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('')
  return `${hex.slice(0, 8)}-${hex.slice(8, 12)}-${hex.slice(12, 16)}-${hex.slice(16, 20)}-${hex.slice(20)}`
}

export const crypto = { getRandomValues, randomUUID }
