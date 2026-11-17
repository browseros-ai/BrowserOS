/**
 * Escaping: names, texts, urls and placeholders that need quoting or escaping.
 */

import type { AriaNodeJSON } from '../../src/prelude/vendored'
import {
  calls,
  element,
  type Gap,
  playwright,
  type Snapshot,
  tree,
} from './model'

const MARKS = [...' !"#$%&\'()*+,-./:;<=>?@[\\]^_`{|}~']

const placed = MARKS.flatMap((mark) => [`${mark}a`, `a${mark}a`, `a${mark}`])

const range = (from: number, to: number) =>
  String.fromCharCode(
    ...Array.from({ length: to - from + 1 }, (_, index) => from + index),
  )

const buttons = (names: string[]): AriaNodeJSON[] =>
  names.map((name, index) => ({ role: 'button', name, ref: `e${index + 1}` }))

const words = (texts: string[]): Snapshot =>
  tree([{ role: 'paragraph', children: texts }])

const regions = (groups: string[][]): Snapshot =>
  tree(groups.map((children) => ({ role: 'region', children })))

const YAML_WORDS = ['y', 'n', 'yes', 'no', 'true', 'false', 'on', 'off', 'null']

const cased = (word: string) => [
  ...new Set([
    word,
    word[0].toUpperCase() + word.slice(1),
    word.toUpperCase(),
    [...word].map((c, i) => (i % 2 ? c.toUpperCase() : c)).join(''),
  ]),
]

// Each ASCII mark at the start, inside and at the end of a name, a url, a placeholder and a text, each field on its own line.
const marks = tree(
  placed.map((each) => ({
    role: 'link',
    name: each,
    url: each,
    placeholder: each,
    text: each,
  })),
)

// The same marks as text pieces: Neo's format joins them into one line and quotes it once.
const marksJoined = words(placed)

// Pieces that need quoting alone but not joined, and the reverse: Neo's format quotes the joined line once.
const joined = regions([
  ['a:', 'b'],
  ['x', '#y'],
  ['12', 'apples'],
  ['yes', 'please'],
  ['-', 'a'],
  ['"hi"', 'there'],
  ['say "hi"', 'and \\ go'],
  ['a\tb', 'c:'],
  ['a ', 'b'],
  ['', 'a', ''],
])

// Control characters: C0, DEL and C1 quote a value and are escaped in it, a tab inside alone does not; a name's C0 is JSON-escaped, its DEL and C1 stay raw in a single-quoted key.
const controls = tree([
  {
    role: 'paragraph',
    children: [
      range(0x00, 0x1f),
      range(0x7f, 0x9f),
      'a\tb',
      'a\nb',
      'a\rb',
      'a\x0bb',
      'a\x1bb',
      'a\x7fb',
      'a\x85b',
      'a\x9fb',
      '\ta',
      'a\t',
    ],
  },
  { role: 'button', name: range(0x00, 0x1f), ref: 'e1' },
  { role: 'button', name: range(0x7f, 0x9f), ref: 'e2' },
  { role: 'link', ref: 'e3', url: `https://a.example/${range(0x00, 0x1f)}` },
  { role: 'textbox', ref: 'e4', placeholder: `Name${range(0x7f, 0x9f)}` },
])

// JavaScript's whitespace has U+FEFF and not U+0085: it decides the quote at either end, after a colon and before a hash.
const spaces = tree([
  {
    role: 'paragraph',
    children: [
      '\u0085a',
      'a\u0085a',
      'a\u0085',
      '\uFEFFa',
      'a\uFEFFa',
      'a\uFEFF',
      '\u2028a',
      'a\u2028a',
      'a\u2028',
      '\u2029a',
      'a\u2029a',
      'a\u2029',
      '\u00A0a',
      'a\u00A0a',
      'a\u00A0',
      'a:\uFEFFb',
      'a:\u2028b',
      'a:\u00A0b',
      'a:\u200Bb',
      'a\uFEFF#b',
      'a\u00A0#b',
      'a\t#b',
      'a\u200B#b',
    ],
  },
  ...buttons([
    '\uFEFFa',
    'a\uFEFF',
    '\u0085a',
    'a\u0085',
    '\u2028a',
    'a\u2029',
    'a:\u2028b',
    'a\uFEFF#b',
  ]),
])

// A whitespace text alone is dropped by Neo's format as separators only; U+0085 is not whitespace and stays.
const spaceAlone = regions([
  ['\u0085'],
  ['\uFEFF'],
  ['\u2028'],
  ['\u2029'],
  ['\u00A0'],
  ['\u3000'],
  [' '],
])

// `JSON.stringify` of a name, every unit below U+0100 and U+2028, U+2029: the short escapes, `\u00xx`, the rest raw; a key that then needs quoting is single-quoted, a `'` doubled.
const jsonNames = tree(
  buttons([
    range(0x00, 0x1f),
    range(0x20, 0x3f),
    range(0x40, 0x5f),
    range(0x60, 0x7f),
    range(0x80, 0x9f),
    range(0xa0, 0xbf),
    range(0xc0, 0xdf),
    range(0xe0, 0xff),
    'a\u2028b\u2029c',
    "it's {here}",
    "it's here",
    'a: b',
    'a #b',
  ]),
)

// YAML's nine words in any case are quoted; U+212A and U+0130 lower into no word, and `~` is not quoted.
const yamlWords = tree([
  {
    role: 'paragraph',
    children: [
      ...YAML_WORDS.flatMap(cased),
      '\u212A',
      '\u0130',
      'o\u212A',
      '\u0130n',
      '~',
      'nil',
      'yes!',
      'nulls',
    ],
  },
  { role: 'link', ref: 'e1', url: 'null' },
  { role: 'textbox', ref: 'e2', placeholder: 'Off' },
])

// A text `Number()` reads as a number is quoted; one it reads as NaN is not.
const numbers = tree([
  {
    role: 'paragraph',
    children: [
      '12',
      '0',
      '00',
      '007',
      '1e3',
      '1E3',
      '1e+3',
      '1e-3',
      '1e1000',
      '0x1F',
      '0X1f',
      '0b1',
      '0B1',
      '0o7',
      '0O7',
      '.5',
      '5.',
      '0.',
      '+1',
      '+.5',
      'Infinity',
      '+Infinity',
      '-Infinity',
      ' 12 ',
      '\u00A012',
      '',
      '   ',
      'NaN',
      'infinity',
      '+0x1F',
      '0x',
      '0x1G',
      '0b2',
      '0o8',
      '1_000',
      '1n',
      '1,000',
      '1 2',
      '1e',
      'e3',
      '.',
      '\u2212' + '1',
      '\uFF11\uFF12',
      '\u0661\u0662',
    ],
  },
  { role: 'link', ref: 'e1', url: '0x1F' },
  { role: 'textbox', ref: 'e2', placeholder: '1e3' },
])

// The empty string in each field: a name prints nothing, a text, a url and a placeholder print `""`.
const empty = tree([
  { role: 'button', name: '', ref: 'e1' },
  { role: 'paragraph', ref: 'e2', text: '' },
  { role: 'link', name: 'Home', ref: 'e3', url: '' },
  { role: 'link', ref: 'e4', url: '' },
  { role: 'textbox', name: 'Name', ref: 'e5', placeholder: '' },
  { role: 'paragraph', children: [''] },
])

export const escaping: Gap = {
  snapshots: {
    'escaping/marks': marks,
    'escaping/marks-joined': marksJoined,
    'escaping/joined': joined,
    'escaping/controls': controls,
    'escaping/spaces': spaces,
    'escaping/space-alone': spaceAlone,
    'escaping/json-names': jsonNames,
    'escaping/yaml-words': yamlWords,
    'escaping/numbers': numbers,
    'escaping/empty': empty,
  },
  cases: [
    ...calls('escaping/marks', { playwright: playwright() }),
    ...calls('escaping/marks-joined', { element: element() }),
    ...calls('escaping/joined', {
      playwright: playwright(),
      element: element(),
    }),
    ...calls('escaping/controls', {
      playwright: playwright(),
      'element-urls': element({ urls: true }),
    }),
    ...calls('escaping/spaces', {
      playwright: playwright(),
      element: element(),
    }),
    ...calls('escaping/space-alone', {
      playwright: playwright(),
      element: element(),
    }),
    ...calls('escaping/json-names', {
      playwright: playwright(),
      element: element(),
    }),
    ...calls('escaping/yaml-words', {
      playwright: playwright(),
      element: element(),
    }),
    ...calls('escaping/numbers', {
      playwright: playwright(),
      element: element(),
    }),
    ...calls('escaping/empty', {
      playwright: playwright(),
      element: element(),
      'element-urls': element({ urls: true }),
    }),
  ],
}
