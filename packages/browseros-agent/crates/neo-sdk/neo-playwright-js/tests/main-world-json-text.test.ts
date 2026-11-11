// json-text.ts spells the main-world function's JSON without the page's globals; Rust's strict
// parse reads what it writes, and it reads what the prelude's JSON.stringify wrote. Both must
// agree with the realm's own JSON byte for byte on every value of Playwright's format.
import { expect, it } from 'bun:test'
import { readJson, writeJson } from '../src/main-world/json-text'

const TRICKY_STRINGS = [
  '',
  'plain',
  'quote " and backslash \\ and slash /',
  '\u0000\u0001\u0008\t\n\u000b\f\r\u001f\u007f',
  'emoji 😀 pair',
  'lone high \ud800 and lone low \udc00, reversed \udc00\ud800',
  'ends high \ud83d',
  '   separators',
  'é ü 中文',
]

const VALUES: unknown[] = [
  null,
  true,
  false,
  0,
  -1,
  1.5,
  1e21,
  5e-324,
  Number.MAX_SAFE_INTEGER,
  ...TRICKY_STRINGS,
  [],
  {},
  [1, 'two', null, [true, { k: 'k', v: 3 }]],
  {
    o: [{ k: 'n', v: { a: [{ v: 'undefined' }, { bi: '10' }], id: 2 } }],
    id: 1,
  },
  { e: { n: 'Error', m: 'boom', s: 'Error: boom\n    at <anonymous>' } },
  { ta: { b: 'AQID', k: 'ui8' } },
  Object.fromEntries(TRICKY_STRINGS.map((text, index) => [text, index])),
]

it('tests that writeJson spells every value as JSON.stringify does', () => {
  for (const value of VALUES)
    expect(writeJson(value)).toBe(JSON.stringify(value))
})

it('tests that writeJson drops what JSON.stringify drops', () => {
  expect(writeJson(undefined)).toBeUndefined()
  expect(writeJson(() => 1)).toBeUndefined()
  expect(writeJson({ a: undefined, b: 1, c: () => 1 })).toBe('{"b":1}')
  expect(writeJson([undefined, () => 1])).toBe('[null,null]')
  expect(writeJson([Number.NaN, Number.POSITIVE_INFINITY])).toBe('[null,null]')
})

it('tests that readJson reads back what JSON.stringify wrote', () => {
  for (const value of VALUES)
    expect(readJson(JSON.stringify(value) as string)).toEqual(
      JSON.parse(JSON.stringify(value) as string),
    )
  expect(readJson(' { "a" : [ 1 , 2 ] , "b" : "\\u00e9\\/" } ')).toEqual({
    a: [1, 2],
    b: 'é/',
  })
})

it('tests that readJson makes every member an own data property, __proto__ too', () => {
  const read = readJson('{"__proto__":{"x":1},"k":2}') as Record<
    string,
    unknown
  >
  expect(Object.getPrototypeOf(read)).toBeNull()
  expect(Object.keys(read)).toEqual(['__proto__', 'k'])
  expect(Object.getOwnPropertyDescriptor(read, '__proto__')?.value).toEqual({
    x: 1,
  })
})

it('tests that readJson keeps lone surrogates as JSON.parse does', () => {
  const text = JSON.stringify('a\ud800b\udc00')
  expect(readJson(text)).toBe(JSON.parse(text))
})

it('tests that readJson refuses text that is not JSON', () => {
  for (const text of [
    '',
    '{',
    '[1,]',
    '{"a":1,}',
    'tru',
    '"\\x"',
    '"\\u12g4"',
    '1 2',
    '-',
    '- 1',
    '1 .5',
    '1.',
    '.5',
    '+1',
    '01',
    '1e',
    '"open',
  ])
    expect(() => readJson(text)).toThrow(SyntaxError)
})
