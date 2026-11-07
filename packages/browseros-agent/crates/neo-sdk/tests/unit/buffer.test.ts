/**
 * The binary transport's realm end (`src/buffer.ts`): `toBinary` and `Buffer.fromBinary` round
 * trip every length tail and every byte value, agree with the base64 Node writes (Playwright's
 * protocol `binary`), and refuse text that is not base64. The Rust end is pinned by
 * neo-runtime-spec's `tests/wire.rs`; both sides use standard base64 with padding.
 */

import { describe, expect, test } from 'bun:test'
import { Buffer as NeoBuffer, toBinary } from '../../src/buffer'

/** `length` bytes counting up from `start`, wrapping at 256. */
function bytes(length: number, start = 0): Uint8Array {
  return Uint8Array.from({ length }, (_, i) => (start + i) & 0xff)
}

describe('binary transport', () => {
  test('tests that empty and 1, 2 and 3 byte tails round trip with Node base64', () => {
    for (const length of [0, 1, 2, 3, 4, 5, 6, 7]) {
      const original = bytes(length, 250)
      const text = toBinary(original)
      expect(text).toBe(Buffer.from(original).toString('base64'))
      const back = NeoBuffer.fromBinary(text)
      expect(back).toBeInstanceOf(NeoBuffer)
      expect([...back]).toEqual([...original])
    }
    expect(toBinary(new Uint8Array())).toBe('')
    expect(NeoBuffer.fromBinary('').length).toBe(0)
  })

  test('tests that all 256 byte values round trip', () => {
    const all = bytes(256)
    const text = toBinary(all)
    expect(text).toBe(Buffer.from(all).toString('base64'))
    expect([...NeoBuffer.fromBinary(text)]).toEqual([...all])
    // A Buffer is a Uint8Array: it goes back out unchanged.
    expect(toBinary(NeoBuffer.fromBinary(text))).toBe(text)
  })

  test('tests that a decoded view of a larger buffer encodes only its own bytes', () => {
    const whole = bytes(16)
    expect(toBinary(whole.subarray(5, 8))).toBe(
      Buffer.from([5, 6, 7]).toString('base64'),
    )
  })

  test('tests that text that is not base64 is refused', () => {
    for (const text of ['%%%', 'QU*D', 'QUJD=QUJD']) {
      expect(() => NeoBuffer.fromBinary(text)).toThrow(
        'neo-sdk: invalid base64 from Rust',
      )
    }
  })

  test('tests that base64 and base64url toString match Node', () => {
    const tricky = Uint8Array.from([0xfb, 0xff, 0xbf, 0x3e, 0x3f])
    const ours = NeoBuffer.fromBinary(toBinary(tricky))
    expect(ours.toString('base64')).toBe(Buffer.from(tricky).toString('base64'))
    expect(ours.toString('base64url')).toBe(
      Buffer.from(tricky).toString('base64url'),
    )
  })
})
