import { describe, expect, test } from 'bun:test'
import { child, DER_TAG, parseDer } from '../src/firefox/der'

describe('parseDer', () => {
  test('rejects a long-form length cut off by the end of the buffer', () => {
    // OCTET STRING with a 2-byte length, but only 1 length byte present.
    expect(() => parseDer(Uint8Array.from([0x04, 0x82, 0x01]))).toThrow(
      'DER: truncated length',
    )
    // The same cut inside a SEQUENCE must fail too, not yield an empty child.
    expect(() =>
      parseDer(Uint8Array.from([0x30, 0x03, 0x04, 0x82, 0x01])),
    ).toThrow('DER: truncated length')
  })

  test('reads a long-form length that is complete', () => {
    const body = new Uint8Array(200).fill(0xab)
    const node = parseDer(
      Uint8Array.from([0x30, 0x81, 203, 0x04, 0x81, 200, ...body]),
    )
    expect(child(node, 0, DER_TAG.OCTET_STRING).value).toEqual(body)
  })
})
