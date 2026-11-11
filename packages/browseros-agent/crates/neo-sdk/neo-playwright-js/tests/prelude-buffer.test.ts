// The prelude's Buffer against Node's, the one upstream's scripts are written for. The oracle
// is this test runtime's own Buffer: on every input below Bun's and Node 26's agree byte for
// byte and message for message (measured once with the same corpus under both). Each check
// runs the script's `Buffer` parameter in a fresh prelude realm and compares what comes out.
import { beforeAll, expect, it } from 'bun:test'
import { loadPrelude, SEED } from './support/prelude-runtime'

const NodeBuffer = globalThis.Buffer
const runtime = loadPrelude({ seed: SEED })
beforeAll(() => runtime.ready)
const inRealm = (code: string): unknown =>
  JSON.parse(String(runtime.evaluate(`JSON.stringify(${code})`)))
const literal = (value: unknown) => JSON.stringify(value)

// What an expression gives with `Buffer` bound: its value, a view as its class and bytes, or
// what it threw (name, Node's code, message).
const OUTCOME = `(work) => {
  try {
    const value = work()
    return { ok: value instanceof Uint8Array ? { [value.constructor.name]: [...value] } : value }
  } catch (error) {
    return { threw: [error.name, error.code, error.message] }
  }
}`
const ours = (code: string): unknown => inRealm(`(${OUTCOME})(() => ${code})`)
const node = (code: string): unknown =>
  JSON.parse(
    JSON.stringify(
      new Function('Buffer', `return (${OUTCOME})(() => ${code})`)(NodeBuffer),
    ),
  )
/** Each expression's outcome in the prelude's realm and with Node's Buffer, keyed by it. */
const sameAsNode = (cases: string[]) =>
  expect(Object.fromEntries(cases.map((code) => [code, ours(code)]))).toEqual(
    Object.fromEntries(cases.map((code) => [code, node(code)])),
  )

it('reads PNG and WebP integer fields with Node bounds, including sliced buffers', () => {
  for (const method of [
    'readUInt8',
    'readUInt16BE',
    'readUInt16LE',
    'readUInt32BE',
    'readUInt32LE',
    'readInt32BE',
  ]) {
    const offsets = [
      '',
      'undefined',
      '0',
      '-0',
      '1',
      '2',
      '3',
      '4',
      '-1',
      '0.5',
      'NaN',
      'Infinity',
      '-Infinity',
      '"0"',
      'null',
      '{}',
    ]
    sameAsNode(
      [
        'Buffer.from([0xff, 0xfe, 0x80, 0x01, 0x7f])',
        'Buffer.from([9, 0xff, 0xfe, 0x80, 0x01, 8]).subarray(1, 5)',
        'Buffer.alloc(0)',
        'Buffer.alloc(2)',
      ].flatMap((buffer) =>
        offsets.map((offset) => `${buffer}.${method}(${offset})`),
      ),
    )
  }
})

const STRINGS: [string, string][] = [
  ['hello', 'utf8'],
  ['héllo €𝄞', 'utf8'],
  ['\ud800', 'utf8'],
  ['a\udc00b', 'utf8'],
  ['😀', 'utf-8'],
  ['aGVsbG8=', 'base64'],
  ['aGVs bG8\n', 'base64'],
  ['aGVsbG8', 'base64'],
  ['aG', 'base64'],
  ['a', 'base64'],
  ['a-_b', 'base64'],
  ['a-_b', 'base64url'],
  ['aGVs$$bG8=', 'base64'],
  ['aGVsbG8=aGVsbG8=', 'base64'],
  ['=aGVs', 'base64'],
  ['abcd', 'hex'],
  ['abc', 'hex'],
  ['abzz12', 'hex'],
  ['AbCd', 'hex'],
  ['zz', 'hex'],
  ['héllo', 'latin1'],
  ['€', 'latin1'],
  ['héllo', 'binary'],
  ['héllo', 'ascii'],
  ['héllo €', 'utf16le'],
  ['x', 'UTF8'],
  ['x', 'Base64'],
]

const BYTES: number[][] = [
  [0x68, 0x69],
  [0xc3, 0xa9],
  [0xc3],
  [0xe2, 0x82],
  [0xe2, 0x82, 0xac],
  [0xf0, 0x9d, 0x84, 0x9e],
  [0xf0, 0x9d, 0x84],
  [0xed, 0xa0, 0x80],
  [0xc0, 0xaf],
  [0xff, 0xfe, 0x41],
  [0xe0, 0x80, 0x80],
  [0xf4, 0x90, 0x80, 0x80],
  [0x80, 0x41],
  [0xe2, 0x41, 0x42],
  [0xf0, 0x9d, 0x41],
  [],
  [0, 1, 2, 250, 251, 252, 253, 254, 255],
  [0x41, 0x00, 0x42, 0x00, 0x43],
]

const ENCODINGS = [
  'utf8',
  'hex',
  'base64',
  'base64url',
  'latin1',
  'ascii',
  'utf16le',
]

it("tests that Buffer.from(string, encoding) gives Node's bytes", () => {
  const ours = STRINGS.map(([text, encoding]) =>
    inRealm(`[...Buffer.from(${literal(text)}, ${literal(encoding)})]`),
  )
  const node = STRINGS.map(([text, encoding]) => [
    ...NodeBuffer.from(text, encoding as BufferEncoding),
  ])
  expect(ours).toEqual(node)
})

it("tests that toString(encoding) gives Node's text, U+FFFD where UTF-8 breaks", () => {
  for (const encoding of ENCODINGS) {
    const ours = BYTES.map((bytes) =>
      inRealm(`Buffer.from(${literal(bytes)}).toString(${literal(encoding)})`),
    )
    const node = BYTES.map((bytes) =>
      NodeBuffer.from(bytes).toString(encoding as BufferEncoding),
    )
    expect({ encoding, ours }).toEqual({ encoding, ours: node })
  }
  expect(inRealm("Buffer.from('hello').toString('utf8', 1, 3)")).toBe('el')
  expect(inRealm("Buffer.from('hello').toString()")).toBe('hello')
})

it("tests that text longer than one string chunk is still Node's, in every encoding, both ways", () => {
  // Lengths on both sides of where each encoding's text fills a chunk, and well past it.
  for (const length of [
    4097, 6143, 6144, 6145, 8191, 8192, 8193, 16_385, 20_002,
  ]) {
    const bytes = Array.from({ length }, (_, index) => (index * 7 + 13) % 256)
    for (const encoding of ENCODINGS) {
      const ours = inRealm(
        `Buffer.from(${literal(bytes)}).toString(${literal(encoding)})`,
      )
      const theirs = NodeBuffer.from(bytes).toString(encoding as BufferEncoding)
      expect({ length, encoding, same: ours === theirs }).toEqual({
        length,
        encoding,
        same: true,
      })
    }
  }
  // Surrogate pairs at every alignment, so some straddle a chunk, and lone surrogates.
  const text = 'x\u{1d11e}\u00e9\ud800'.repeat(4000)
  for (const encoding of ENCODINGS) {
    const written = NodeBuffer.from(text).toString(encoding as BufferEncoding)
    const ours = inRealm(
      `[...Buffer.from(${literal(written)}, ${literal(encoding)})]`,
    )
    const theirs = [...NodeBuffer.from(written, encoding as BufferEncoding)]
    expect({ encoding, same: literal(ours) === literal(theirs) }).toEqual({
      encoding,
      same: true,
    })
  }
  expect(
    inRealm(
      `Buffer.from(${literal(text)}).toString() === ${literal(NodeBuffer.from(text).toString())}`,
    ),
  ).toBe(true)
})

it('tests that arrays, array-likes, ArrayBuffers and objects convert as in Node', () => {
  expect(inRealm('[...Buffer.from([256, -1, 1.7, NaN, "7", null])]')).toEqual([
    ...NodeBuffer.from([256, -1, 1.7, Number.NaN, '7', null] as never),
  ])
  expect(inRealm('[...Buffer.from(new Uint16Array([0x1234, 0xff]))]')).toEqual([
    52, 255,
  ])
  expect(inRealm("[...Buffer.from({ type: 'Buffer', data: [1, 2] })]")).toEqual(
    [1, 2],
  )
  expect(inRealm("[...Buffer.from(new String('ab'))]")).toEqual([97, 98])
  expect(
    inRealm(
      '(() => { const memory = new Uint8Array([9, 8, 7, 6]).buffer; const view = Buffer.from(memory, 1, 2); new Uint8Array(memory)[1] = 42; return [...view] })()',
    ),
  ).toEqual([42, 7])
})

it("tests that a bad argument throws Node's TypeError, code and message", () => {
  const thrown = (code: string) =>
    inRealm(
      `(() => { try { ${code}; return null } catch (e) { return [e.name, e.code, e.message] } })()`,
    )
  const node = (work: () => unknown) => {
    try {
      work()
      return null
    } catch (error) {
      const failure = error as { name: string; code: string; message: string }
      return [failure.name, failure.code, failure.message]
    }
  }
  for (const [code, value] of [
    ['5', 5],
    ['undefined', undefined],
    ['null', null],
    ['true', true],
    ['{}', {}],
  ] as const)
    expect(thrown(`Buffer.from(${code})`)).toEqual(
      node(() => NodeBuffer.from(value as never)),
    )
  expect(thrown("Buffer.from('x', 'nope')")).toEqual(
    node(() => NodeBuffer.from('x', 'nope' as BufferEncoding)),
  )
  expect(thrown("Buffer.from([1]).toString('nope')")).toEqual(
    node(() => NodeBuffer.from([1]).toString('nope' as BufferEncoding)),
  )
})

it('tests that the static helpers and equals answer as in Node', () => {
  expect(
    inRealm(
      "[[...Buffer.concat([Buffer.from('a'), new Uint8Array([98]), Buffer.from('c')])], [...Buffer.concat([Buffer.from('abc')], 2)], [...Buffer.concat([Buffer.from('a')], 3)]]",
    ),
  ).toEqual([
    [97, 98, 99],
    [97, 98],
    [97, 0, 0],
  ])
  expect(
    inRealm(
      "[Buffer.byteLength('héllo'), Buffer.byteLength('aGVsbG8=', 'base64'), Buffer.byteLength(new Uint8Array(7))]",
    ),
  ).toEqual([6, 5, 7])
  expect(
    inRealm(
      "[[...Buffer.alloc(3)], [...Buffer.alloc(5, 'ab')], [...Buffer.alloc(3, 7)]]",
    ),
  ).toEqual([
    [0, 0, 0],
    [97, 98, 97, 98, 97],
    [7, 7, 7],
  ])
  expect(
    inRealm(
      "[Buffer.isBuffer(Buffer.from('a')), Buffer.isBuffer(new Uint8Array(1)), Buffer.isEncoding('UTF-8'), Buffer.isEncoding('nope')]",
    ),
  ).toEqual([true, false, true, false])
  expect(
    inRealm(
      "(() => { const b = Buffer.from('hello'); const s = b.slice(1, 3); s[0] = 69; return [b.toString(), Buffer.isBuffer(s), Buffer.isBuffer(b.subarray(1)), Buffer.from('ab').equals(Buffer.from('ab')), Buffer.from('ab').equals(new Uint8Array([97, 99]))] })()",
    ),
  ).toEqual(['hEllo', true, true, true, false])
})

it('tests that a Buffer is a Uint8Array, so the codec still sends it as the bytes brand', () => {
  expect(
    inRealm(
      "Buffer.from('hi') instanceof Uint8Array && Buffer.from('hi').length === 2",
    ),
  ).toBe(true)
  expect(inRealm("JSON.stringify(Buffer.from('hi'))")).toBe('{"0":104,"1":105}')
})

it("tests that screenshot bytes revive as a Buffer whose toString('base64') is base64", async () => {
  const shot = loadPrelude({
    seed: SEED,
    answer: () => ({ $neo: 'bytes', base64: 'iVBORw0KGgo=' }),
  })
  expect(
    await shot.run(
      "const bytes = await page.screenshot(); return [Buffer.isBuffer(bytes), bytes.toString('base64'), [...bytes.subarray(0, 4)]]",
    ),
  ).toEqual([true, 'iVBORw0KGgo=', [137, 80, 78, 71]])
})

it("tests that only Node's encoding names are encodings, not names every object inherits", () => {
  sameAsNode([
    "Buffer.isEncoding('constructor')",
    "Buffer.isEncoding('__proto__')",
    "Buffer.isEncoding('hasOwnProperty')",
    "Buffer.isEncoding('UCS-2')",
    "Buffer.isEncoding('')",
    "Buffer.from('x', 'constructor')",
    "Buffer.from([1]).toString('__proto__')",
  ])
})

it("tests that fill writes Node's bytes over Node's range, and refuses what Node refuses", () => {
  sameAsNode([
    "Buffer.alloc(3).fill('x')",
    "Buffer.alloc(4).fill('ab', 1)",
    "Buffer.alloc(4).fill('ab', 1, 3)",
    "Buffer.alloc(4).fill('6162', 'hex')",
    "Buffer.alloc(4).fill('6162', 1, 'hex')",
    "Buffer.alloc(5).fill('aGk=', 'base64')",
    "Buffer.alloc(5).fill('\\u20ac')",
    "Buffer.alloc(5).fill('\\u20ac', 'latin1')",
    "Buffer.alloc(5).fill('\\u00e9', 'utf16le')",
    "Buffer.alloc(2).fill('abcdef')",
    "Buffer.alloc(4).fill('', 'hex')",
    "Buffer.alloc(4).fill('x', '')",
    "Buffer.alloc(4).fill('x', 0, 4, null)",
    "Buffer.alloc(4).fill('x', undefined, 2)",
    'Buffer.alloc(4).fill(1, undefined, 2)',
    "Buffer.alloc(4).fill('x', 5)",
    "Buffer.alloc(4).fill('x', -0)",
    'Buffer.alloc(4).fill(new Uint16Array([0x0102]))',
    "Buffer.alloc(4).fill(Buffer.from('ab'), 1)",
    'Buffer.alloc(4).fill(true)',
    'Buffer.alloc(4).fill(null)',
    'Buffer.alloc(4).fill(257)',
    'Buffer.alloc(4).fill(-1)',
    'Buffer.alloc(4).fill(1.7)',
    'Buffer.alloc(4).fill({ valueOf: () => 65 })',
    "Buffer.alloc(9).fill('abc', 1, 8)",
    "Buffer.alloc(4).fill('zz', 'hex')",
    "Buffer.alloc(4).fill('z', 'hex')",
    "Buffer.alloc(4).fill(\"z'\", 'hex')",
    "Buffer.alloc(4).fill('a\\nb', 'hex')",
    "Buffer.alloc(4).fill('zz', 2, 2, 'hex')",
    "Buffer.alloc(4).fill('x', 0, 5)",
    "Buffer.alloc(4).fill('x', 0, -1)",
    "Buffer.alloc(4).fill('x', 1.5)",
    "Buffer.alloc(4).fill('x', NaN)",
    "Buffer.alloc(4).fill('x', '1')",
    "Buffer.alloc(4).fill(1, '1')",
    "Buffer.alloc(4).fill('x', 0, 'nope')",
    "Buffer.alloc(4).fill('x', 0, 4, 5)",
    "Buffer.alloc(3, 'zz', 'hex')",
    "Buffer.alloc(3, '')",
    "Buffer.alloc(3, '\\u00e9', 'latin1')",
    "Buffer.alloc(3, 'ab', 'nope')",
    "Buffer.alloc(0, 'zz', 'hex')",
  ])
  // Bun's Buffer differs from Node's here (its MAX_LENGTH is 2 ** 32, it words an empty fill
  // differently, and it skips the fill of a size under 1), so these are Node 26's own answers.
  const outOfRange = (name: string, received: string) => ({
    threw: [
      'RangeError',
      'ERR_OUT_OF_RANGE',
      `The value of "${name}" is out of range. It must be >= 0 && <= 9007199254740991. Received ${received}`,
    ],
  })
  const invalid = (received: string) => ({
    threw: [
      'TypeError',
      'ERR_INVALID_ARG_VALUE',
      `The argument 'value' is invalid. Received ${received}`,
    ],
  })
  expect({
    negative: ours("Buffer.alloc(4).fill('x', -1)"),
    past: ours("Buffer.alloc(4).fill('x', 2 ** 53)"),
    farOffset: ours("Buffer.alloc(4).fill('x', 2 ** 33)"),
    emptyView: ours('Buffer.alloc(4).fill(new Uint8Array(0))'),
    emptyBuffer: ours('Buffer.alloc(4).fill(Buffer.alloc(0))'),
    allocEmpty: ours('Buffer.alloc(3, Buffer.alloc(0))'),
    allocNegative: ours('Buffer.alloc(-1)'),
    allocUnderOne: ours("Buffer.alloc(0.5, 'x', 'nope')"),
  }).toEqual({
    negative: outOfRange('offset', '-1'),
    past: outOfRange('offset', '9_007_199_254_740_992'),
    farOffset: { ok: { Buffer: [0, 0, 0, 0] } },
    emptyView: invalid('Uint8Array(0) []'),
    emptyBuffer: invalid('<Buffer >'),
    allocEmpty: invalid('<Buffer >'),
    allocNegative: outOfRange('size', '-1'),
    allocUnderOne: {
      threw: ['TypeError', 'ERR_UNKNOWN_ENCODING', 'Unknown encoding: nope'],
    },
  })
})

it("tests that indexOf, lastIndexOf and includes search as Node's, strings and buffers included", () => {
  const hello = "Buffer.from('hello')"
  const wide = "Buffer.from('h\\u00e9llo', 'utf16le')"
  sameAsNode([
    `${hello}.indexOf('l')`,
    `${hello}.lastIndexOf('l')`,
    `${hello}.includes('ell')`,
    `${hello}.includes('elk')`,
    `${hello}.indexOf(Buffer.from('lo'))`,
    `${hello}.indexOf(new Uint8Array([108, 108]))`,
    `${hello}.indexOf(0x6c + 256)`,
    `${hello}.indexOf(108.9)`,
    `${hello}.indexOf(-148)`,
    `${hello}.indexOf('l', 3)`,
    `${hello}.indexOf('l', -2)`,
    `${hello}.indexOf('l', -20)`,
    `${hello}.indexOf('l', 20)`,
    `${hello}.indexOf('l', 1.9)`,
    `${hello}.indexOf('l', -1.9)`,
    `${hello}.indexOf('l', Infinity)`,
    `${hello}.indexOf('l', null)`,
    `${hello}.indexOf('l', {})`,
    `${hello}.lastIndexOf('l', {})`,
    `${hello}.lastIndexOf('l', 2)`,
    `${hello}.lastIndexOf('l', 20)`,
    `${hello}.lastIndexOf('l', -20)`,
    `${hello}.lastIndexOf('l', -Infinity)`,
    `${hello}.lastIndexOf(Buffer.from('lo'), 1)`,
    `${hello}.indexOf('')`,
    `${hello}.indexOf('', 20)`,
    `${hello}.indexOf('', -2)`,
    `${hello}.lastIndexOf('')`,
    "Buffer.from('').indexOf('')",
    "Buffer.from('').indexOf('a')",
    "Buffer.from('').lastIndexOf(0)",
    `${hello}.indexOf('l', 0, 2)`,
    `${hello}.indexOf('l', 0, 3)`,
    `${hello}.indexOf('l', 0, -1)`,
    `${hello}.indexOf('l', 0, 10)`,
    `${hello}.indexOf('l', 0, Infinity)`,
    `${hello}.indexOf('l', 0, 2.9)`,
    `${hello}.indexOf('', 0, 2)`,
    `${hello}.indexOf(108, 0, 3)`,
    `${hello}.lastIndexOf('l', 4, 3)`,
    `${hello}.lastIndexOf(108, undefined, 3)`,
    `${hello}.includes('lo', 0, 4)`,
    `${hello}.indexOf('6c6c', 'hex')`,
    `${hello}.indexOf('6c6c', 0, 'hex')`,
    `${hello}.indexOf('6c6c', 0, 5, 'hex')`,
    `${hello}.indexOf('bGw=', 'base64')`,
    `${hello}.indexOf('bGw', 'base64url')`,
    `${hello}.indexOf('zz', 'hex')`,
    `${hello}.indexOf('l', 'latin1')`,
    `${hello}.indexOf('l', 'ascii')`,
    "Buffer.from('h\\u00e9llo', 'latin1').indexOf('\\u00e9', 'latin1')",
    "Buffer.from('h\\u00e9llo').indexOf('\\u00e9')",
    "Buffer.from('h\\u00e9llo').indexOf('\\u00e9', 'binary')",
    `${hello}.indexOf('l', 0, 'nope')`,
    `${hello}.indexOf('l', '')`,
    `${hello}.indexOf(Buffer.from('l'), 0, 'nope')`,
    `${hello}.indexOf({})`,
    `${hello}.indexOf(null)`,
    `${hello}.indexOf(new Uint16Array([0x6c6c]))`,
    `${hello}.indexOf(Symbol.iterator)`,
    `${wide}.indexOf('l', 0, 'utf16le')`,
    `${wide}.indexOf('l', 5, 'utf16le')`,
    `${wide}.lastIndexOf('l', 0, 'ucs2')`,
    `${wide}.lastIndexOf('l', 'ucs2')`,
    `${wide}.indexOf('l', 0, 7, 'utf16le')`,
    `${wide}.indexOf('\\u00e9l', 'utf-16le')`,
    "Buffer.from([0x6c, 0, 0x6c, 0, 0x6c]).indexOf('l', 3, 'utf16le')",
    "Buffer.from([0x6c, 0, 0x6c, 0, 0x6c]).lastIndexOf('l', 'utf16le')",
    "Buffer.from([1, 0, 2, 0, 3]).indexOf(Buffer.from([9, 0]), 0, 'utf16le')",
    "Buffer.from([1, 0, 2, 0, 3]).indexOf(Buffer.from([2, 0]), 0, 'utf16le')",
    "Buffer.from([0, 1, 0, 2]).indexOf(Buffer.from([1, 0]), 0, 'utf16le')",
    'Buffer.from([0, 1, 0, 2]).indexOf(Buffer.from([1, 0]))',
    "Buffer.from([1, 0]).indexOf(Buffer.from([1]), 0, 'utf16le')",
    "Buffer.from([1, 0, 2, 0]).indexOf(Buffer.from([2, 0, 9]), 0, 'ucs2')",
  ])
  // Node 26 ends a search at 0 for an `end` of NaN, where Bun's Buffer searches it all.
  expect(ours(`${hello}.indexOf('l', 0, NaN)`)).toEqual({ ok: -1 })
})

it("tests that the rest of Uint8Array's members a Buffer answers differently are Node's", () => {
  const hello = "Buffer.from('hello')"
  sameAsNode([
    `${hello}.toLocaleString()`,
    `${hello}.toLocaleString('hex')`,
    `${hello}.toLocaleString('base64', 1, 3)`,
    "new Buffer('ab')",
    "new Buffer('aGk=', 'base64')",
    "new Buffer('6162', 'hex')",
    "new Buffer('ab', 'nope')",
    'new Buffer(2)',
    'new Buffer([1, 2])',
    'new Buffer(new Uint8Array([1, 2, 3]).buffer, 1, 1)',
    `new Buffer(${hello})`,
    "Buffer.isBuffer(new Buffer('ab'))",
    `${hello}.subarray('1', '3')`,
    `${hello}.subarray(NaN, null)`,
    `${hello}.subarray(-3, -1)`,
    `${hello}.slice(1.7, 3.2)`,
    `${hello}.slice(-Infinity, Infinity)`,
    `(() => { const b = ${hello}; b.slice(1, 2)[0] = 69; return b })()`,
    `${hello}.map((byte) => byte + 1)`,
    `${hello}.with(0, 72)`,
    'Buffer.of(1, 256, -1)',
  ])
})
