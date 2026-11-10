/**
 * Writes `tests/aria-yaml-goldens.json`: what Playwright's own YAML renderer, vendored unchanged
 * in `vendor/playwright/isomorphic/`, prints for a corpus of aria snapshots and tricky strings.
 * neo-runtime's `src/aria_yaml.rs` ports `renderAriaSnapshotAsYaml` (called with no options, as
 * Playwright's server calls it) and `yaml.ts`'s escaping to Rust, and its tests decode every
 * snapshot below with the generated `AriaNode` types and compare the YAML byte for byte.
 *
 * Usage, from crates/neo-playwright-injected:
 *   bun scripts/aria-yaml-goldens.ts           # rewrites the JSON
 *   bun scripts/aria-yaml-goldens.ts --check   # fails when the committed JSON is stale
 *
 * `bun run test` runs `--check`, so neo-check fails on a corpus edited without regenerating.
 * Playwright's server renders under V8 and this runs under JavaScriptCore; the corpus has no
 * case where the two engines may differ (no lone surrogates, which serde cannot decode, and no
 * locale-sensitive call). Bundled for Node (`bun build --target=node`), the script reproduces
 * the committed file under V8 too.
 *
 * Every snapshot uses only `AriaNodeJSON` fields with values Playwright's injected script can
 * produce in shape (a role from `AriaRole`, integral `level` and `box`), because Rust decodes
 * each one with `deny_unknown_fields`.
 */

import { readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'
import type { AriaNodeJSON } from '../vendor/playwright/isomorphic/ariaSnapshot'
import { renderAriaSnapshotAsYaml } from '../vendor/playwright/isomorphic/ariaSnapshotRenderer'
import {
  yamlEscapeKeyIfNeeded,
  yamlEscapeValueIfNeeded,
} from '../vendor/playwright/isomorphic/yaml'

const out = join(
  dirname(fileURLToPath(import.meta.url)),
  '../tests/aria-yaml-goldens.json',
)

/**
 * Strings for the escaping functions, and as names and texts below. Grouped by the rule of
 * `yamlStringNeedsQuotes` they probe, each with near misses that must stay unquoted.
 */
const strings: string[] = [
  // Empty, and JS `\s` at either end: every WhiteSpace and LineTerminator code point, plus
  // near misses that are not `\s` (U+0085, U+180E, U+200B).
  '',
  ' ',
  '  ',
  'a',
  'a ',
  ' a',
  '\ta',
  'a\t',
  '\na',
  'a\n',
  '\u000ba',
  'a\u000c',
  '\ra',
  '\u00a0a',
  'a\u00a0',
  '\u1680a',
  '\u2000a',
  'a\u2005',
  '\u200aa',
  '\u2028a',
  'a\u2029',
  '\u202fa',
  '\u205fa',
  '\u3000a',
  '\ufeffa',
  'a\ufeff',
  '\u0085a',
  '\u180ea',
  'a\u180e',
  '\u200ba',
  'a\u200b',
  // Control characters: quoted for \x00-\x08, \x0b, \x0c, \x0e-\x1f, \x7f-\x9f; a tab inside
  // stays unquoted, as does a code point just past the range.
  'a\u0000b',
  'a\u0001b',
  'a\u0007b',
  'a\u0008b',
  'a\tb',
  'a\u000bb',
  'a\u000cb',
  'a\u000eb',
  'a\u001bb',
  'a\u001fb',
  'a\u007fb',
  'a\u0080b',
  'a\u0085b',
  'a\u009fb',
  'a\u00a0b',
  'a\u00a1b',
  '\u0000',
  '\u007f',
  // A leading dash.
  '-',
  '-a',
  '- a',
  '--',
  'a-',
  'a - b',
  '\u2212a',
  '+a',
  // A colon or newline followed by `\s` or the end.
  ':',
  'a:',
  'a: b',
  'a:b',
  'a:\tb',
  'a:\u00a0b',
  'a:\u3000b',
  'a::',
  'http://a.test/x',
  'a:\u200bb',
  'a\n',
  'a\nb',
  'key: value',
  // `#` after `\s`.
  '#',
  '#a',
  '# a',
  'a#',
  'a#b',
  'a #',
  'a #b',
  'a\t#b',
  'a\u00a0#b',
  'a\u2028#b',
  'C#',
  // Line breaks anywhere.
  'a\rb',
  'a\r\nb',
  // Indicator characters and quotes at the start, and the same characters elsewhere.
  '&a',
  'a&',
  '*a',
  'a*',
  ']a',
  'a]',
  ',a',
  'a,',
  '?a',
  'a?',
  '!a',
  'a!',
  '>a',
  'a>',
  '|a',
  'a|',
  '@a',
  'a@b.c',
  '"a',
  'a"',
  "'a",
  "a'",
  '%a',
  'a%',
  '"',
  "'",
  '""',
  "''",
  // Braces and backticks anywhere; a bracket only at the start.
  '{',
  'a{b',
  'a}b',
  '}',
  '`',
  'a`b',
  '[',
  '[a]',
  'a[b]',
  'a]b',
  // Strings `Number()` reads as a number, and near misses it reads as NaN.
  '0',
  '1',
  '42',
  '007',
  '00',
  '+1',
  '+0',
  '1.5',
  '.5',
  '5.',
  '0.0',
  '1e3',
  '1E3',
  '1e+3',
  '1e-3',
  '1.e3',
  '0e0',
  '1e1000',
  '5e-324',
  '9007199254740993',
  '0x1f',
  '0X1F',
  '0xff',
  '0b101',
  '0B1',
  '0o17',
  '0O7',
  'Infinity',
  '+Infinity',
  '1e',
  '1e+',
  'e3',
  '.e3',
  '.',
  '+',
  '+.5',
  '0x',
  '0xg',
  '0x_1',
  '+0x1',
  '0b',
  '0b2',
  '0o',
  '0o8',
  '00x1',
  '1_000',
  '1,000',
  '1.2.3',
  '12n',
  'NaN',
  'nan',
  'infinity',
  'INFINITY',
  'Inf',
  '\u0661\u0662\u0663',
  '\uff11\uff12\uff13',
  // YAML's other scalars, any case; near misses stay unquoted.
  'y',
  'Y',
  'n',
  'N',
  'yes',
  'Yes',
  'YES',
  'no',
  'No',
  'true',
  'True',
  'TRUE',
  'tRuE',
  'false',
  'False',
  'on',
  'On',
  'ON',
  'off',
  'Off',
  'null',
  'Null',
  'NULL',
  '~',
  '~a',
  'nil',
  'none',
  'yess',
  'nul',
  '\uff59\uff45\uff53',
  '.inf',
  '.Inf',
  '.INF',
  '-.inf',
  '+.inf',
  '+.INF',
  '.nan',
  '.NaN',
  '.NAN',
  '+.nan',
  '..inf',
  '.infinity',
  '.in',
  'inf',
  // Escaping inside double quotes: backslashes and quotes alone need none.
  'a\\b',
  '\\',
  'a\\nb',
  "it's",
  'say "hi"',
  'say "hi": now',
  "it's: here",
  '- "quoted" \\ and \u0001',
  '\b\f\n\r\t',
  // Unicode passes through.
  'h\u00e9llo',
  '\u65e5\u672c\u8a9e',
  '\u{1f600}',
  'a\u{1f600}b',
  '\u{1f600}: x',
  'e\u0301',
  '\u00df',
  '\u212a',
  '\u0130',
  '\u{10ffff}',
  '\u00ad',
  '</script>',
  'Submit',
  'Sign in to your account',
]

/** A string of `count` repeats of `unit`. */
const repeat = (unit: string, count: number) => unit.repeat(count)

/**
 * Names around Playwright's 900-unit key limit, counted in UTF-16 code units: an astral
 * character counts twice, so 450 emoji fit and 451 do not.
 */
const longNames = [
  repeat('a', 899),
  repeat('a', 900),
  repeat('a', 901),
  repeat('\u{1f600}', 450),
  repeat('\u{1f600}', 451),
  repeat('\u00e9', 900),
  repeat('\u00e9', 901),
  repeat('a', 2000),
]

/** Snapshots, each a case name and Playwright's `AriaSnapshotJSON`. */
const snapshots: [string, AriaNodeJSON[]][] = [
  ['empty snapshot', []],
  ['one leaf', [{ role: 'button', name: 'Submit' }]],
  ['a leaf without a name', [{ role: 'separator' }]],
  ['an empty name is no name', [{ role: 'button', name: '' }]],
  ['top-level text', [{ role: 'text', text: 'hello world' }]],
  ['top-level text, empty', [{ role: 'text', text: '' }]],
  ['top-level text, absent', [{ role: 'text' }]],
  ['a text role ignores its name', [{ role: 'text', name: 'n', text: 't' }]],
  ['leaf with text', [{ role: 'paragraph', text: 'Hello' }]],
  ['leaf with empty text', [{ role: 'paragraph', text: '' }]],
  [
    'leaf with text and a name',
    [{ role: 'heading', name: 'Title', level: 1, text: 'Title text' }],
  ],
  [
    'text wins over children',
    [
      {
        role: 'paragraph',
        text: 'x',
        children: [{ role: 'button', name: 'hidden' }],
      },
    ],
  ],
  ['empty children is a leaf', [{ role: 'list', children: [] }]],
  ['an empty string child', [{ role: 'listitem', children: [''] }]],
  [
    'string and node children',
    [
      {
        role: 'paragraph',
        children: [
          'Read the ',
          { role: 'link', name: 'docs', url: '/docs' },
          ' first.',
        ],
      },
    ],
  ],
  ['url prop', [{ role: 'link', name: 'Home', url: 'https://a.test/' }]],
  ['empty url prop', [{ role: 'link', name: 'Home', url: '' }]],
  [
    'url prop with text',
    [{ role: 'link', name: 'Home', url: '/', text: 'Go home' }],
  ],
  [
    'url prop with children',
    [
      {
        role: 'link',
        name: 'Card',
        url: '/card?id=1&x=2',
        children: [{ role: 'img', name: 'Photo' }, 'caption'],
      },
    ],
  ],
  [
    'placeholder prop',
    [{ role: 'textbox', name: 'Email', placeholder: 'you@example.com' }],
  ],
  [
    'url and placeholder props',
    [{ role: 'textbox', url: 'x: y', placeholder: '- dash', text: 't' }],
  ],
  [
    'props that need quotes',
    [
      {
        role: 'link',
        name: 'a',
        url: 'https://a.test/#frag ment',
        placeholder: '123',
      },
    ],
  ],
  [
    'checked',
    [
      { role: 'checkbox', name: 'a', checked: true },
      { role: 'checkbox', name: 'b', checked: 'mixed' },
      { role: 'checkbox', name: 'c', checked: false as unknown as true },
    ],
  ],
  [
    'disabled, expanded, active',
    [
      { role: 'button', name: 'a', disabled: true },
      { role: 'button', name: 'b', disabled: false as unknown as true },
      { role: 'combobox', name: 'c', expanded: true },
      { role: 'combobox', name: 'd', expanded: false as unknown as true },
      { role: 'textbox', name: 'e', active: true },
      { role: 'textbox', name: 'f', active: false as unknown as true },
    ],
  ],
  [
    'invalid',
    [
      { role: 'textbox', name: 'a', invalid: true },
      { role: 'textbox', name: 'b', invalid: 'grammar' },
      { role: 'textbox', name: 'c', invalid: 'spelling' },
      { role: 'textbox', name: 'd', invalid: false as unknown as true },
    ],
  ],
  [
    'levels',
    [
      { role: 'heading', name: 'a', level: 1 },
      { role: 'heading', name: 'b', level: 6 },
      { role: 'heading', name: 'c', level: 0 },
      { role: 'treeitem', name: 'd', level: 42 },
      { role: 'treeitem', name: 'e', level: -1 },
    ],
  ],
  [
    'pressed and selected',
    [
      { role: 'button', name: 'a', pressed: true },
      { role: 'button', name: 'b', pressed: 'mixed' },
      { role: 'button', name: 'c', pressed: false as unknown as true },
      { role: 'tab', name: 'd', selected: true },
      { role: 'tab', name: 'e', selected: false as unknown as true },
    ],
  ],
  [
    'aria-hidden',
    [
      { role: 'generic', ariaHidden: true, text: 'hidden' },
      { role: 'generic', ariaHidden: false as unknown as true, text: 'shown' },
    ],
  ],
  [
    'refs and cursors',
    [
      { role: 'button', name: 'a', ref: 'e1' },
      { role: 'button', name: 'b', ref: 'e2', cursor: 'pointer' },
      { role: 'button', name: 'c', cursor: 'pointer' },
      { role: 'button', name: 'd', ref: '', cursor: 'pointer' },
      { role: 'button', name: 'e', ref: 'f12e345' },
    ],
  ],
  [
    'boxes',
    [
      {
        role: 'button',
        name: 'a',
        box: { x: 8, y: 16, width: 100, height: 20 },
      },
      { role: 'button', name: 'b', box: { x: -5, y: 0, width: 0, height: 0 } },
      {
        role: 'button',
        name: 'c',
        ref: 'e3',
        cursor: 'pointer',
        box: { x: 1, y: 2, width: 3, height: 4 },
      },
    ],
  ],
  [
    'every flag, in Playwright order',
    [
      {
        role: 'treeitem',
        name: 'All',
        checked: 'mixed',
        disabled: true,
        expanded: true,
        active: true,
        invalid: 'spelling',
        level: 3,
        pressed: 'mixed',
        selected: true,
        ariaHidden: true,
        ref: 'e9',
        cursor: 'pointer',
        box: { x: 0, y: 1, width: 2, height: 3 },
        url: 'u',
        placeholder: 'p',
        text: 'txt',
      },
    ],
  ],
  [
    'iframe nodes render as they are',
    [
      { role: 'iframe', ref: 'e4' },
      { role: 'iframe', ref: 'e5', cursor: 'pointer', text: 'fallback' },
      { role: 'iframe' },
    ],
  ],
  [
    'nested lists',
    [
      {
        role: 'list',
        children: [
          {
            role: 'listitem',
            children: [
              'One',
              {
                role: 'list',
                children: [
                  { role: 'listitem', text: 'One.a' },
                  {
                    role: 'listitem',
                    children: [
                      { role: 'link', name: 'One.b', url: '/b', ref: 'e7' },
                    ],
                  },
                ],
              },
            ],
          },
          { role: 'listitem', text: 'Two' },
        ],
      },
    ],
  ],
  [
    'several top-level nodes',
    [
      {
        role: 'banner',
        children: [{ role: 'heading', name: 'Site', level: 1 }],
      },
      { role: 'main', children: [{ role: 'paragraph', text: 'Body' }] },
      { role: 'contentinfo', text: '\u00a9 2026' },
    ],
  ],
  [
    'a form, as an ai snapshot prints it',
    [
      {
        role: 'generic',
        ref: 'e1',
        children: [
          { role: 'heading', name: 'Sign in', level: 2, ref: 'e2' },
          {
            role: 'textbox',
            name: 'Email',
            ref: 'e3',
            placeholder: 'you@example.com',
            active: true,
          },
          { role: 'textbox', name: 'Password', ref: 'e4' },
          { role: 'checkbox', name: 'Remember me', checked: true, ref: 'e5' },
          { role: 'button', name: 'Sign in', ref: 'e6', cursor: 'pointer' },
          {
            role: 'link',
            name: 'Forgot password?',
            ref: 'e7',
            cursor: 'pointer',
            url: '/reset',
          },
        ],
      },
    ],
  ],
  [
    'a table',
    [
      {
        role: 'table',
        children: [
          {
            role: 'rowgroup',
            children: [
              {
                role: 'row',
                name: 'Name Qty',
                children: [
                  { role: 'columnheader', name: 'Name' },
                  { role: 'columnheader', name: 'Qty' },
                ],
              },
            ],
          },
          {
            role: 'rowgroup',
            children: [
              {
                role: 'row',
                name: 'Apples 3',
                children: [
                  { role: 'cell', name: 'Apples' },
                  { role: 'cell', name: '3' },
                ],
              },
            ],
          },
        ],
      },
    ],
  ],
  ['deep nesting', [deep(12)]],
  [
    'keys that need single quotes',
    strings.map((name) => ({ role: 'button', name })),
  ],
  [
    'texts that need double quotes',
    [
      {
        role: 'list',
        children: strings.map((text) => ({ role: 'listitem', text })),
      },
    ],
  ],
  [
    'string children that need quotes',
    [{ role: 'paragraph', children: strings }],
  ],
  [
    'props over every string',
    [
      {
        role: 'list',
        children: strings.map((url) => ({ role: 'link', name: 'x', url })),
      },
    ],
  ],
  [
    'names around the 900-unit limit',
    longNames.map((name) => ({ role: 'button', name })),
  ],
  [
    'long texts are never cut',
    [{ role: 'paragraph', text: `${repeat('word ', 300)}end` }],
  ],
]

/** `depth` nested groups around one button. */
function deep(depth: number): AriaNodeJSON {
  let node: AriaNodeJSON = { role: 'button', name: 'Deepest', ref: 'e99' }
  for (let level = depth; level > 0; level--)
    node = {
      role: 'group',
      name: `level ${level}`,
      children: [node, `after ${level}`],
    }
  return node
}

const corpus = {
  source:
    'vendor/playwright/isomorphic/ariaSnapshotRenderer.ts and yaml.ts at Playwright 26137face',
  snapshots: snapshots.map(([name, nodes]) => ({
    name,
    nodes,
    yaml: renderAriaSnapshotAsYaml(nodes),
  })),
  strings: strings.map((input) => ({
    input,
    key: yamlEscapeKeyIfNeeded(input),
    value: yamlEscapeValueIfNeeded(input),
  })),
}

const text = `${JSON.stringify(corpus, null, 2)}\n`
if (process.argv.includes('--check')) {
  // Compared as parsed JSON: the repo's formatter rewrites the committed file's layout, never
  // its values.
  let committed: unknown
  try {
    committed = JSON.parse(readFileSync(out, 'utf8'))
  } catch {}
  if (JSON.stringify(committed) !== JSON.stringify(corpus)) {
    console.error(
      `${out} is stale: run \`bun scripts/aria-yaml-goldens.ts\` in crates/neo-sdk/neo-playwright-injected`,
    )
    process.exit(1)
  }
  console.log(
    `aria-yaml-goldens: ${corpus.snapshots.length} snapshots and ${corpus.strings.length} strings match`,
  )
} else {
  writeFileSync(out, text)
  console.log(
    `wrote ${out}: ${corpus.snapshots.length} snapshots, ${corpus.strings.length} strings`,
  )
}
