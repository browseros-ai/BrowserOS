// The mark follows the value (ruling R50) by a keyed digest written here with 32-bit halves,
// because the browser's own digest call answers later than the mask can wait. These rows pin it
// to SipHash-2-4's reference vectors (key 00..0f, message 00..n-1), so an edit that breaks a
// lane's carry or rotation fails here rather than masking the wrong boxes on a page.
import { describe, expect, it } from 'bun:test'
import { digest, setAside, sipHash24 } from '../src/probe/private-digest'

const KEY = new Uint32Array([0x03020100, 0x07060504, 0x0b0a0908, 0x0f0e0d0c])

const hex = ([lo, hi]: [number, number]) =>
  hi.toString(16).padStart(8, '0') + lo.toString(16).padStart(8, '0')

const counting = (length: number) =>
  Uint8Array.from({ length }, (_, index) => index)

describe('the R50 digest', () => {
  it('is SipHash-2-4 by its reference vectors', () => {
    // Lengths 0, 8 and 63 cross the empty, the whole-block and the longest partial last block.
    expect(hex(sipHash24(KEY, counting(0)))).toBe('726fdb47dd0e0e31')
    expect(hex(sipHash24(KEY, counting(8)))).toBe('93f5f5799a932462')
    expect(hex(sipHash24(KEY, counting(15)))).toBe('a129ca6149be45e5')
    expect(hex(sipHash24(KEY, counting(63)))).toBe('958a324ceb064572')
  })

  it('depends on the key and on every character', () => {
    const other = new Uint32Array([1, 2, 3, 4])
    const card = '4242424242424242'
    expect(digest(KEY, card)).toBe(digest(KEY, card))
    expect(digest(other, card)).not.toBe(digest(KEY, card))
    expect(digest(KEY, '4242424242424243')).not.toBe(digest(KEY, card))
  })

  it('sets aside whitespace, dashes and slashes, as `spaced` does', () => {
    expect(setAside('4242 4242-4242/4242\t')).toBe('4242424242424242')
    expect(setAside(' - / ')).toBe('')
  })
})
