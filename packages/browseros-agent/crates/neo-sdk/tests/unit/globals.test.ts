/**
 * The standard globals neo-runtime defines from `src/globals.ts`, against Bun's own (Node's
 * semantics): `URL`/`URLSearchParams` linkage over the stub parser (`url-stub.ts`), `Buffer`'s
 * statics, `TextEncoder`/`TextDecoder`, `structuredClone` and `crypto`. The parity file
 * (`tests/parity/globals.test.mjs`) runs the same globals in the real realm against Node.
 */

import { beforeEach, describe, expect, test } from 'bun:test'
import { Buffer as NeoBuffer } from '../../src/buffer'
import { standardGlobals } from '../../src/globals'
import { structuredClone as neoClone } from '../../src/structured-clone'
import {
  TextDecoder as NeoDecoder,
  TextEncoder as NeoEncoder,
} from '../../src/text'
import { URLSearchParams as NeoParams, URL as NeoURL } from '../../src/url'
import { crypto as neoCrypto } from '../../src/web-crypto'
import { answerURL } from './url-stub'

beforeEach(() => {
  ;(globalThis as Record<string, unknown>).__neo_runtime = {
    get(request: string): string {
      const asked = JSON.parse(request)
      const url = answerURL(asked)
      if (url !== undefined) return url
      if (asked.field === 'randomValues') {
        const bytes = new Uint8Array(asked.length)
        globalThis.crypto.getRandomValues(bytes)
        return JSON.stringify(Buffer.from(bytes).toString('base64'))
      }
      throw new Error(`unexpected getter ${request}`)
    },
  }
})

test('the realm gets exactly the globals it lacks', () => {
  expect(Object.keys(standardGlobals()).sort()).toEqual(
    [
      'Buffer',
      'TextDecoder',
      'TextEncoder',
      'URL',
      'URLSearchParams',
      'crypto',
      'structuredClone',
    ].sort(),
  )
  expect(standardGlobals().Buffer).toBe(NeoBuffer)
})

describe('URL', () => {
  test('a URL keeps its searchParams and its query in step both ways', () => {
    const url = new NeoURL('/p?a=1', 'https://h.test/x/')
    const params = url.searchParams
    params.append('b', '2 3')
    expect(url.href).toBe('https://h.test/p?a=1&b=2+3')
    url.search = '?c=%C3%A9'
    expect(url.searchParams).toBe(params)
    expect([...params]).toEqual([['c', 'é']])
    params.delete('c')
    expect(url.href).toBe('https://h.test/p')
    url.href = 'https://o.test/?k=v'
    expect(params.get('k')).toBe('v')
  })

  test('a URL that does not parse is a TypeError, and parse/canParse do not throw', () => {
    expect(() => new NeoURL('not a url')).toThrow(TypeError)
    expect(() => {
      new NeoURL('https://h.test/').href = 'nope'
    }).toThrow(TypeError)
    expect(NeoURL.parse('nope')).toBeNull()
    expect(NeoURL.canParse('/x', 'https://h.test/')).toBe(true)
  })

  // `null` is left out: Node reads it as no init, where Bun reads it as the string 'null'.
  test('URLSearchParams builds, edits and serializes as Node does', () => {
    const inits: unknown[] = [
      '?a=1&b=%zz&c=a+b&&d',
      { a: '1 2', b: "!'()~*" },
      [
        ['x', '1'],
        ['x', '2'],
      ],
      undefined,
    ]
    for (const init of inits) {
      const ours = new NeoParams(init)
      const theirs = new URLSearchParams(init as string)
      ours.set('x', 'é')
      theirs.set('x', 'é')
      ours.append('z', '\ud800')
      theirs.append('z', '\ud800')
      ours.sort()
      theirs.sort()
      expect(ours.toString()).toBe(theirs.toString())
      expect([...ours]).toEqual([...theirs])
    }
  })
})

describe('Buffer', () => {
  test('from, alloc, concat and byteLength agree with Node', () => {
    const cases: [unknown, string?][] = [
      ['héllo €😀'],
      ['aMOp bGxv\n', 'base64'],
      ['aGk_-w', 'base64url'],
      ['68656c6c6fzz', 'hex'],
      ['héllo', 'latin1'],
      ['hi', 'utf16le'],
      ['a\ud800b'],
      [[1, 2, 256, -1]],
      [new Uint16Array([1, 258])],
      [{ type: 'Buffer', data: [104, 105] }],
    ]
    for (const [value, encoding] of cases) {
      const ours = NeoBuffer.from(value, encoding)
      const theirs = Buffer.from(value as string, encoding as BufferEncoding)
      expect(ours).toBeInstanceOf(NeoBuffer)
      expect(ours.toString('hex')).toBe(theirs.toString('hex'))
      if (typeof value === 'string')
        expect(NeoBuffer.byteLength(value, encoding)).toBe(theirs.length)
    }
    expect(NeoBuffer.alloc(5, 'ab').toString()).toBe('ababa')
    expect(NeoBuffer.alloc(3, 257).toString('hex')).toBe('010101')
    expect(() => NeoBuffer.alloc(-1)).toThrow(RangeError)
    const parts = [NeoBuffer.from('ab'), new Uint8Array([99])]
    expect(NeoBuffer.concat(parts).toString()).toBe('abc')
    expect(NeoBuffer.concat(parts, 5).toString('hex')).toBe('6162630000')
    expect(() => NeoBuffer.from('a', 'nope')).toThrow(TypeError)
  })

  test('a Buffer over an ArrayBuffer shares its memory', () => {
    const memory = new ArrayBuffer(4)
    NeoBuffer.from(memory, 1, 2)[0] = 9
    expect(new Uint8Array(memory)[1]).toBe(9)
  })
})

test('TextEncoder and TextDecoder round trip UTF-8 as Node does', () => {
  const text = 'héllo €😀 a\ud800b'
  const bytes = new NeoEncoder().encode(text)
  expect([...bytes]).toEqual([...new TextEncoder().encode(text)])
  const invalid = new Uint8Array([
    0xef, 0xbb, 0xbf, 0x61, 0xff, 0xe2, 0x82, 0x62, 0xf0, 0x9f,
  ])
  expect(new NeoDecoder().decode(invalid)).toBe(
    new TextDecoder().decode(invalid),
  )
  expect(() =>
    new NeoDecoder('utf-8', { fatal: true }).decode(invalid),
  ).toThrow(TypeError)
  const stream = new NeoDecoder()
  const euro = new NeoEncoder().encode('€😀')
  const chunks = [euro.subarray(0, 2), euro.subarray(2, 5), euro.subarray(5)]
  expect(
    chunks.map((chunk, i) => stream.decode(chunk, { stream: i < 2 })).join(''),
  ).toBe('€😀')
  expect(() => new NeoDecoder('latin1')).toThrow(RangeError)
})

test('structuredClone keeps types, sharing and cycles, and refuses functions', () => {
  const shared = { x: 1 }
  const value: Record<string, unknown> = {
    date: new Date(5),
    map: new Map([[1, shared]]),
    set: new Set([shared]),
    regexp: /a/gi,
    bytes: NeoBuffer.from('hi'),
  }
  value.self = value
  const copy = neoClone(value) as typeof value
  expect(copy.self).toBe(copy)
  expect((copy.map as Map<number, unknown>).get(1)).toBe(
    [...(copy.set as Set<unknown>)][0],
  )
  expect(copy.date).toEqual(new Date(5))
  expect(Object.prototype.toString.call(copy.bytes)).toBe('[object Uint8Array]')
  expect(copy.bytes).not.toBeInstanceOf(NeoBuffer)
  expect(() => neoClone(() => 1)).toThrow(
    expect.objectContaining({ name: 'DataCloneError' }),
  )
})

test('crypto draws its bytes from Rust', () => {
  expect(neoCrypto.randomUUID()).toMatch(
    /^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$/,
  )
  const values = neoCrypto.getRandomValues(new Uint32Array(4))
  expect(values).toBeInstanceOf(Uint32Array)
  expect(() => neoCrypto.getRandomValues(new Uint8Array(65537))).toThrow(
    expect.objectContaining({ name: 'QuotaExceededError' }),
  )
})
