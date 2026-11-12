// The v1 snapshot text (src/prelude/snapshot-v1.ts), from the snapshot op's `{ json, partial }`.
// The blind page's JSON was recorded from a live engine by neo-playwright's prelude_live test,
// which also checks a live page prints the same text; the rule tests use small trees.
import { expect, it } from 'bun:test'
import { readFileSync } from 'node:fs'
import { renderSnapshotV1 } from '../src/prelude/snapshot-v1'
import type { SnapshotResult } from '../src/prelude/wire'
import { loadPrelude, SEED } from './support/prelude-runtime'

const blindData = (name: string): string =>
  readFileSync(
    new URL(
      `../../neo-playwright/tests/data/snapshot-v1/${name}`,
      import.meta.url,
    ),
    'utf8',
  )
const blind = JSON.parse(blindData('blind.json')) as SnapshotResult

it('tests that the blind page prints its expected text by default and with text: false', async () => {
  const runtime = loadPrelude({
    seed: SEED,
    answer: (call) => (call.name === 'page.ariaSnapshot' ? blind : null),
  })
  expect(await runtime.run('return await page.ariaSnapshot()')).toBe(
    blindData('blind.v1-default.txt'),
  )
  expect(
    await runtime.run(
      "return await page.ariaSnapshot({ mode: 'ai', text: false })",
    ),
  ).toBe(blindData('blind.v1-interactive.txt'))
  expect(renderSnapshotV1(blind, { urls: false, maxTokens: 8_000 })).toBe(
    blindData('blind.v1-default.txt'),
  )
})

type Node = SnapshotResult['json'][number]
type Child = Node | string

const render = (
  json: Node[],
  options: { text?: boolean; urls?: boolean; maxTokens?: number } = {},
  partial: SnapshotResult['partial'] = [],
): string =>
  renderSnapshotV1(
    { json, partial },
    { urls: false, maxTokens: 8_000, ...options },
  )
const both = (json: Node[]): [string, string] => [
  render(json, { text: true }),
  render(json, { text: false }),
]

const button = (name: string, ref: string): Node => ({
  role: 'button',
  name,
  ref,
})
const generic = (children: Child[], extra: Partial<Node> = {}): Node => ({
  role: 'generic',
  children,
  ...extra,
})

it('tests rule 1: text pieces at one level join into one line, quoted once, and only with text on', () => {
  const json = [
    generic(['Price', generic([], { text: '#1 in Toothpaste' })]),
    button('Buy', 'e2'),
    generic(['*']),
  ]
  expect(both(json)).toEqual([
    '- text: "Price #1 in Toothpaste"\n- button "Buy" [ref=e2]\n- text: "*"',
    '- button "Buy" [ref=e2]',
  ])
})

const link = (name: string, ref: string): Node => ({ role: 'link', name, ref })

it('tests rule 1: a text line of separators alone is dropped, judged on the joined line, so a split price stays', () => {
  // Hacker News's story line: `|` between links, the site in parentheses.
  const json = [
    generic(['|']),
    link('hide', 'e1'),
    generic(['|']),
    link('198 comments', 'e2'),
    generic(['(']),
    link('github.com/aws', 'e3'),
    generic([')', '586 points by']),
    button('Buy', 'e4'),
    // A price arrives in pieces; `$` or `.` alone would be dropped, the joined line is not.
    generic(['$', '7', '.', '49']),
    button('Add', 'e5'),
    generic(['·', ' – ', '»', '{ }', '\\', '‐']),
    button('Next', 'e6'),
    generic(['#', '|']),
  ]
  expect(both(json)).toEqual([
    '- link "hide" [ref=e1]\n- link "198 comments" [ref=e2]\n- link "github.com/aws" [ref=e3]\n' +
      '- text: ) 586 points by\n- button "Buy" [ref=e4]\n- text: $ 7 . 49\n- button "Add" [ref=e5]\n' +
      '- button "Next" [ref=e6]\n- text: "# |"',
    '- link "hide" [ref=e1]\n- link "198 comments" [ref=e2]\n- link "github.com/aws" [ref=e3]\n' +
      '- button "Buy" [ref=e4]\n- button "Add" [ref=e5]\n- button "Next" [ref=e6]',
  ])
})

it('tests rule 1: a block holding only separator lines is empty, and the text around it joins', () => {
  const json = [
    generic(['Before']),
    // `|` and `·` join into one line of separators: the navigation prints nothing.
    section('navigation', 'e1', [generic(['|']), generic(['·'])]),
    generic(['after']),
    { role: 'status', ref: 'e2', children: [generic(['—'])] },
    { role: 'alert', name: 'Notice', ref: 'e3', text: '|' },
    { role: 'iframe', ref: 'e4', children: ['/'] },
    section('region', 'e5', ['$', generic(['5'])]),
  ] as Node[]
  expect(both(json)).toEqual([
    '- text: Before after\n- alert "Notice"\n- iframe [ref=e4]\n- region [ref=e5]:\n  - text: $ 5',
    '- alert "Notice"\n- iframe [ref=e4]',
  ])
})

it('tests rule 2: a kept block prints its text whatever the form, a lone piece in the leaf form', () => {
  const json = [
    { role: 'alert', ref: 'e1', text: 'Invalid password.' },
    {
      role: 'dialog',
      ref: 'e2',
      children: [
        { role: 'paragraph', ref: 'e3', text: 'Delete "Acme"?' },
        button('Delete', 'e4'),
      ],
    },
    {
      role: 'status',
      ref: 'e5',
      children: [generic(['Saved']), generic(['2m ago'])],
    },
    { role: 'status', ref: 'e6' },
    { role: 'alertdialog', name: 'Session ended', ref: 'e7' },
  ] as Node[]
  const printed =
    '- alert: Invalid password.\n- dialog [ref=e2]:\n  - text: Delete "Acme"?\n  - button "Delete" [ref=e4]\n' +
    '- status: Saved 2m ago\n- alertdialog "Session ended" [ref=e7]'
  expect(both(json)).toEqual([printed, printed])
})

it('tests rule 2: an actionable alert or status prints its ref and marker, a dialog always its ref', () => {
  const json = [
    {
      role: 'status',
      ref: 'e19',
      cursor: 'pointer',
      text: 'Saved 2 minutes ago',
    },
    {
      role: 'alert',
      ref: 'e20',
      cursor: 'pointer',
      children: [generic(['Card declined']), button('Retry', 'e21')],
    },
    { role: 'dialog', ref: 'e22', text: 'Uploading…' },
    { role: 'alert', ref: 'e23', text: 'Not clickable' },
  ] as Node[]
  const printed =
    '- status [ref=e19] [cursor=pointer]: Saved 2 minutes ago\n' +
    '- alert [ref=e20] [cursor=pointer]:\n  - text: Card declined\n  - button "Retry" [ref=e21]\n' +
    '- dialog [ref=e22]: Uploading…\n- alert: Not clickable'
  expect(both(json)).toEqual([printed, printed])
})

it('tests rule 3: a section prints its ref and indents its content; an empty one goes, an iframe stays', () => {
  const json = [
    {
      role: 'navigation',
      name: 'Primary',
      ref: 'e1',
      children: [generic(['Menu']), { role: 'link', name: 'Home', ref: 'e2' }],
    },
    { role: 'region', ref: 'e3', children: ['Only text'] },
    { role: 'iframe', ref: 'e4' },
  ] as Node[]
  expect(both(json)).toEqual([
    '- navigation "Primary" [ref=e1]:\n  - text: Menu\n  - link "Home" [ref=e2]\n- region [ref=e3]:\n  - text: Only text\n- iframe [ref=e4]',
    '- navigation "Primary" [ref=e1]:\n  - link "Home" [ref=e2]\n- iframe [ref=e4]',
  ])
})

it('tests rule 4: a heading prints without its ref, its links on their own lines and its text once', () => {
  const json = [
    {
      role: 'heading',
      name: 'Results',
      level: 2,
      ref: 'e1',
      children: [{ role: 'link', name: 'Results', ref: 'e2' }],
    },
    {
      role: 'heading',
      level: 3,
      ref: 'e3',
      children: ['Step 2', { role: 'link', name: 'edit', ref: 'e4' }],
    },
  ] as Node[]
  const printed =
    '- heading "Results" [level=2]\n- link "Results" [ref=e2]\n- heading [level=3]: Step 2\n- link "edit" [ref=e4]'
  expect(both(json)).toEqual([printed, printed])
})

it('tests rule 5: an actionable node is one line with its value, or its own text clipped without text', () => {
  const long =
    'Free delivery on orders over $35 shipped by Amazon, or fastest delivery tomorrow morning'
  const json = [
    { role: 'textbox', name: 'Email', ref: 'e1', text: 'a@b.c' },
    { role: 'button', name: 'Go', ref: 'e2', cursor: 'pointer' },
    generic(['Add to cart'], { ref: 'e3', cursor: 'pointer' }),
    generic([long, button('Details', 'e5')], { ref: 'e4', cursor: 'pointer' }),
    generic(['Plain wrapper'], { ref: 'e6' }),
  ] as Node[]
  expect(long.length).toBeGreaterThan(80)
  const [text, interactive] = both(json)
  expect(text).toBe(
    '- textbox "Email" [ref=e1]: a@b.c\n- button "Go" [ref=e2]\n- generic [ref=e3] [cursor=pointer]: Add to cart\n' +
      `- generic [ref=e4] [cursor=pointer]: ${long}\n- button "Details" [ref=e5]\n- text: Plain wrapper`,
  )
  expect(interactive).toBe(
    '- textbox "Email" [ref=e1]: a@b.c\n- button "Go" [ref=e2]\n- generic [ref=e3] [cursor=pointer]: Add to cart\n' +
      `- generic [ref=e4] [cursor=pointer]: ${long.slice(0, 80).trimEnd()}…\n- button "Details" [ref=e5]`,
  )
})

it("tests rule 5: a nameless control's own text takes an image's alt text, never a nested control's name or a heading's", () => {
  const json = [
    // A link around a badge: the image's alt text is its only label.
    {
      role: 'link',
      ref: 'e685',
      url: 'https://badge.fury.io/js/acme',
      children: [{ role: 'img', name: 'npm version', ref: 'e686' }],
    },
    // The image is the nested button's label, not the link's.
    {
      role: 'link',
      ref: 'e1',
      children: [
        {
          role: 'button',
          ref: 'e2',
          children: [{ role: 'img', name: 'Close' }],
        },
      ],
    },
    generic([button('Follow', 'e4')], { ref: 'e3', cursor: 'pointer' }),
    // The heading prints its own line just below, so the link says only the rest.
    {
      role: 'link',
      ref: 'e5',
      children: [
        { role: 'heading', name: 'Amazon Music', level: 5 },
        generic(['Stream millions of songs']),
      ],
    },
  ] as Node[]
  const printed =
    '- link [ref=e685]: npm version\n- link [ref=e1]\n- button [ref=e2]: Close\n' +
    '- generic [ref=e3] [cursor=pointer]\n- button "Follow" [ref=e4]\n' +
    '- link [ref=e5]: Stream millions of songs\n- heading "Amazon Music" [level=5]'
  expect(both(json)).toEqual([printed, printed])
})

it('tests rule 5: a link with nothing else to say says its url, clipped without text, and not with urls: true', () => {
  const long = `https://en.wikipedia.org/wiki/Special:Search?search=${'a'.repeat(80)}`
  const json = [
    {
      role: 'link',
      ref: 'e11',
      cursor: 'pointer',
      url: 'https://news.ycombinator.com',
    },
    { role: 'link', ref: 'e12', url: long },
    { role: 'link', ref: 'e13', url: '/item', children: ['Read more'] },
    { role: 'link', name: 'Docs', ref: 'e14', url: '/docs' },
    { role: 'button', ref: 'e15' },
  ] as Node[]
  const rest =
    '- link [ref=e13]: Read more\n- link "Docs" [ref=e14]\n- button [ref=e15]'
  expect(both(json)).toEqual([
    `- link [ref=e11]: https://news.ycombinator.com\n- link [ref=e12]: ${long}\n${rest}`,
    `- link [ref=e11]: https://news.ycombinator.com\n- link [ref=e12]: ${long.slice(0, 80)}…\n${rest}`,
  ])
  // With urls: true the `/url` line says where it goes, so the label stays empty.
  expect(render(json.slice(0, 2), { text: false, urls: true })).toBe(
    '- link [ref=e11]:\n  - /url: https://news.ycombinator.com\n' +
      `- link [ref=e12]:\n  - /url: ${long}`,
  )
})

const select = (count: number, selected: number): Node => ({
  role: 'combobox',
  name: 'Quantity:',
  ref: 'e1',
  children: Array.from({ length: count }, (_, index) => ({
    role: 'option',
    name: String(index + 1),
    ...(index === selected ? { selected: true } : {}),
  })) as Node[],
})

it('tests rule 5: a native select with 5 options lists them all in both forms', () => {
  const options = [1, 2, 3, 4, 5].map(
    (n) => `  - option "${n}"${n === 2 ? ' [selected]' : ''}`,
  )
  const printed = ['- combobox "Quantity:" [ref=e1]:', ...options].join('\n')
  expect(both([select(5, 1)])).toEqual([printed, printed])
})

it('tests rule 5: a native select with 30 options lists only the selected one without text', () => {
  const [text, interactive] = both([select(30, 0)])
  expect(text.split('\n')).toHaveLength(31)
  expect(text.split('\n')[30]).toBe('  - option "30"')
  expect(interactive).toBe(
    '- combobox "Quantity:" [ref=e1]:\n  - option "1" [selected]\n  - text: 29 more options',
  )
})

it('tests rule 5: a native select with option groups lists their options, the groups printing no line', () => {
  const group = (name: string, options: string[], selected?: string): Node => ({
    role: 'group',
    name,
    children: options.map((option) => ({
      role: 'option',
      name: option,
      ...(option === selected ? { selected: true } : {}),
    })) as Node[],
  })
  const json = [
    {
      role: 'combobox',
      name: 'Destination',
      ref: 'e1',
      children: [
        group('Europe', ['France', 'Germany'], 'France'),
        group('Asia', ['Japan']),
      ],
    },
  ] as Node[]
  const printed =
    '- combobox "Destination" [ref=e1]:\n  - option "France" [selected]\n  - option "Germany"\n  - option "Japan"'
  expect(both(json)).toEqual([printed, printed])
})

/**
 * Recorded from the engine on a live BrowserOS (ariaSnapshotJSON({ mode: 'ai' })): a product table
 * and a list with a paragraph, a fieldset, an empty named group and a note.
 */
const realTable = [
  {
    children: [
      {
        children: [
          {
            children: [
              { name: 'Item', ref: 'e5', role: 'columnheader' },
              { name: 'Price', ref: 'e6', role: 'columnheader' },
              { ref: 'e7', role: 'columnheader' },
            ],
            ref: 'e4',
            role: 'row',
          },
        ],
        ref: 'e3',
        role: 'rowgroup',
      },
      {
        children: [
          {
            children: [
              { name: 'Sensodyne Repair 3.4oz', ref: 'e10', role: 'cell' },
              { name: '$7.49', ref: 'e11', role: 'cell' },
              {
                children: [{ name: 'Add to cart', ref: 'e13', role: 'button' }],
                ref: 'e12',
                role: 'cell',
              },
            ],
            ref: 'e9',
            role: 'row',
          },
          {
            children: [
              { name: 'Sensodyne 6-pack', ref: 'e15', role: 'cell' },
              { name: '$41.00', ref: 'e16', role: 'cell' },
              {
                children: [{ name: 'Add to cart', ref: 'e18', role: 'button' }],
                ref: 'e17',
                role: 'cell',
              },
            ],
            ref: 'e14',
            role: 'row',
          },
          {
            children: [
              {
                children: [
                  {
                    cursor: 'pointer',
                    name: 'Toothbrush',
                    ref: 'e21',
                    role: 'link',
                    url: '#r',
                  },
                ],
                ref: 'e20',
                role: 'cell',
              },
              { name: '$3.10', ref: 'e22', role: 'cell' },
              { ref: 'e23', role: 'cell' },
            ],
            ref: 'e19',
            role: 'row',
          },
        ],
        ref: 'e8',
        role: 'rowgroup',
      },
    ],
    ref: 'e2',
    role: 'table',
  },
] as Node[]
const realList = [
  {
    active: true,
    children: [
      {
        children: [
          { ref: 'e3', role: 'listitem', text: 'Apples' },
          {
            children: [
              'Pears',
              {
                cursor: 'pointer',
                name: 'details',
                ref: 'e5',
                role: 'link',
                url: '#p',
              },
            ],
            ref: 'e4',
            role: 'listitem',
          },
          {
            children: [
              {
                cursor: 'pointer',
                name: 'Kiwis',
                ref: 'e7',
                role: 'link',
                url: '#k',
              },
            ],
            ref: 'e6',
            role: 'listitem',
          },
        ],
        ref: 'e2',
        role: 'list',
      },
      { ref: 'e8', role: 'paragraph', text: 'Paragraph text' },
      {
        children: [
          {
            children: [
              { name: 'Standard', ref: 'e12', role: 'radio' },
              'Standard',
            ],
            ref: 'e11',
            role: 'generic',
          },
        ],
        name: 'Shipping',
        ref: 'e9',
        role: 'group',
      },
      { name: 'Empty group', role: 'group' },
      { ref: 'e13', role: 'note', text: 'Note text' },
    ],
    ref: 'e1',
    role: 'generic',
  },
] as Node[]

it('tests rule 7: a table cell named by its text keeps the text, and a cell holding a link adds no name', () => {
  // Each plain cell is `cell "$7.49"` with no children: the engine dropped the text that repeated
  // the name. The cell around the Toothbrush link has no name, so nothing prints twice.
  expect(both(realTable)).toEqual([
    '- text: Item Price Sensodyne Repair 3.4oz $7.49\n- button "Add to cart" [ref=e13]\n- text: Sensodyne 6-pack $41.00\n- button "Add to cart" [ref=e18]\n- link "Toothbrush" [ref=e21]\n- text: $3.10',
    '- button "Add to cart" [ref=e13]\n- button "Add to cart" [ref=e18]\n- link "Toothbrush" [ref=e21]',
  ])
})

it("tests rule 7: list items, a paragraph and a named empty group keep their text; a fieldset's name stays out", () => {
  // The fieldset's legend is folded into `group "Shipping"`, which has other children: names of
  // containers with content are not printed in v1 (design section 11).
  expect(both(realList)).toEqual([
    '- text: Apples Pears\n- link "details" [ref=e5]\n- link "Kiwis" [ref=e7]\n- text: Paragraph text\n- radio "Standard" [ref=e12]\n- text: Standard Empty group Note text',
    '- link "details" [ref=e5]\n- link "Kiwis" [ref=e7]\n- radio "Standard" [ref=e12]',
  ])
})

it('tests rule 7: a cell gives its name as text, a row with children does not repeat theirs', () => {
  const json = [
    {
      role: 'row',
      name: 'Toothpaste $7.49',
      ref: 'e1',
      children: [
        { role: 'cell', name: 'Toothpaste', ref: 'e2' },
        { role: 'cell', name: '$7.49', ref: 'e3' },
      ],
    },
  ] as Node[]
  expect(both(json)).toEqual(['- text: Toothpaste $7.49', ''])
})

it('tests rule 6: a named image prints with text on only', () => {
  const json = [
    { role: 'img', name: 'Logo', ref: 'e1' },
    { role: 'img', ref: 'e2' },
  ] as Node[]
  expect(both(json)).toEqual(['- img "Logo"', ''])
})

it('tests rule 7: other roles print no line and lose their refs; their content moves up', () => {
  const json = [
    {
      role: 'list',
      ref: 'e1',
      children: [
        { role: 'listitem', ref: 'e2', text: 'One' },
        {
          role: 'listitem',
          ref: 'e3',
          children: ['Two', { role: 'link', name: 'more', ref: 'e4' }],
        },
      ],
    },
  ] as Node[]
  expect(both(json)).toEqual([
    '- text: One Two\n- link "more" [ref=e4]',
    '- link "more" [ref=e4]',
  ])
})

it('tests that a tree far deeper than any call stack renders: the walk keeps its own stack', () => {
  let deep: Node = button('Deep', 'e1')
  for (let level = 0; level < 100_000; level++)
    deep = generic(
      [level === 0 ? 'Bottom' : '', deep].filter(Boolean) as Child[],
    )
  expect(both([deep])).toEqual([
    '- text: Bottom\n- button "Deep" [ref=e1]',
    '- button "Deep" [ref=e1]',
  ])
})

it("tests that a locator's root node goes through the same rules", () => {
  expect(
    render([{ role: 'main', ref: 'e2', children: [button('Go', 'e3')] }]),
  ).toBe('- main [ref=e2]:\n  - button "Go" [ref=e3]')
  expect(render([button('Go', 'e3')])).toBe('- button "Go" [ref=e3]')
  expect(render([{ role: 'text', text: 'Hello' }])).toBe('- text: Hello')
})

it('tests that urls: true prints each link’s /url, and a placeholder always prints', () => {
  const json = [
    { role: 'link', name: 'Docs', ref: 'e1', url: '/docs' },
    { role: 'textbox', name: 'Search', ref: 'e2', placeholder: 'Type here' },
  ] as Node[]
  expect(render(json)).toBe(
    '- link "Docs" [ref=e1]\n- textbox "Search" [ref=e2]:\n  - /placeholder: Type here',
  )
  expect(render(json, { urls: true })).toBe(
    '- link "Docs" [ref=e1]:\n  - /url: /docs\n- textbox "Search" [ref=e2]:\n  - /placeholder: Type here',
  )
})

it('tests that a partial iframe keeps its line with the comment under it, and a lost one goes last', () => {
  const json = [
    { role: 'main', ref: 'e1', children: [{ role: 'iframe', ref: 'e3' }] },
  ] as Node[]
  const partial = [
    {
      ref: 'e3',
      frame: 'https://pay.example/',
      reason: 'timeout after 2000 ms',
    },
    { ref: 'e9', frame: 'https://gone.example/', reason: 'target closed' },
  ]
  expect(render(json, {}, partial)).toBe(
    '- main [ref=e1]:\n  - iframe [ref=e3]\n    # partial: https://pay.example/ not read (timeout after 2000 ms)\n' +
      '# partial: https://gone.example/ not read (target closed)',
  )
})

/** A button, then one text piece making the text form exactly `length` characters long. */
const sized = (length: number): Node[] => {
  const head = '- button "Go" [ref=e1]\n- text: '
  return [button('Go', 'e1'), generic(['a'.repeat(length - head.length)])]
}

it('tests that the text form is returned up to 3,000 tokens and the notice past it', () => {
  expect(render(sized(12_000))).toBe(render(sized(12_000), { text: true }))
  expect(render(sized(12_000)).length).toBe(12_000)
  // 12,001 - 22 characters of text left out: 2994.75 tokens, about 3000.
  expect(render(sized(12_001))).toBe(
    '# text left out: about 3000 tokens. Pass { text: true } to include it.\n- button "Go" [ref=e1]',
  )
})

it('tests that the notice rounds to the nearest 100 tokens and says at least 100', () => {
  // 13,022 characters with text, 22 without: 3,250 tokens left out, rounded up.
  expect(render(sized(13_022)).split('\n')[0]).toBe(
    '# text left out: about 3300 tokens. Pass { text: true } to include it.',
  )
  const controls = Array.from({ length: 600 }, (_, index) =>
    button('Go', `e${index}`),
  )
  const json = [...controls, generic(['Short'])]
  expect(render(json, { text: true }).length).toBeGreaterThan(12_000)
  expect(render(json).split('\n')[0]).toBe(
    '# text left out: about 100 tokens. Pass { text: true } to include it.',
  )
})

/** `count` buttons with uniform 48-character lines at level 1: `Add item 21 to the basket`. */
const items = (first: number, count: number): Node[] =>
  Array.from({ length: count }, (_, index) =>
    button(`Add item ${first + index} to the basket`, `e${first + index}`),
  )
const section = (
  role: string,
  ref: string,
  children: Child[],
  name?: string,
): Node => ({ role, ref, children, ...(name ? { name } : {}) }) as Node

/** The uncut interactive form's lines. */
const uncut = (json: Node[]): string[] =>
  render(json, { text: false }).split('\n')
/** The maxTokens that keeps exactly the first `count` lines of the interactive form. */
const keeping = (json: Node[], count: number): number =>
  Math.ceil(uncut(json).slice(0, count).join('\n').length / 4)

// The cut line names a section as `the <role> section`, its quoted name after the role when it
// has one; each form is checked with a named and an unnamed section.
for (const [name, named] of [
  [undefined, 'the main section'],
  ['Results', 'the main "Results" section'],
] as const)
  it(`tests the cut line offering the outermost open section that fits the cap: ${named}`, () => {
    const json = [
      section('banner', 'e1', items(10, 6)),
      section('main', 'e20', items(21, 4), name),
      section('contentinfo', 'e40', items(41, 2)),
    ]
    const lines = uncut(json)
    // The banner and two of main's buttons fit; main (5 lines) fits the cap on its own.
    expect(render(json, { text: false, maxTokens: keeping(json, 10) })).toBe(
      [
        ...lines.slice(0, 10),
        `# cut: 5 more lines (5 with refs); the whole snapshot is about 200 tokens. page.getByRef('e20').ariaSnapshot() shows all of ${named}. Or pass a larger maxTokens.`,
      ].join('\n'),
    )
  })

for (const [name, named] of [
  ['Footer', 'the contentinfo "Footer" section'],
  [undefined, 'the contentinfo section'],
] as const)
  it(`tests the cut line offering the next section when the open one is larger than the cap: ${named}`, () => {
    const json = [
      section('main', 'e20', items(21, 12)),
      section('contentinfo', 'e40', items(41, 2), name),
    ]
    const lines = uncut(json)
    expect(render(json, { text: false, maxTokens: keeping(json, 4) })).toBe(
      [
        ...lines.slice(0, 4),
        '# cut: 12 more lines (12 with refs); the whole snapshot is about 200 tokens. ' +
          `page.getByRef('e40').ariaSnapshot() shows ${named}, which is not shown above. Or pass a larger maxTokens.`,
      ].join('\n'),
    )
  })

it('tests that separator lines dropped from the text form leave the offered section and the kept block whole', () => {
  // Six dropped `|` lines sit before main: the cap must still see main's own lines, and print
  // the dialog after the cut from its first line to its last, the button after it counted.
  const bars = (ref: number): Child[] => [
    generic(['|']),
    button(`Tab ${ref}`, `e${ref}`),
  ]
  const json = [
    ...[1, 2, 3, 4, 5].flatMap(bars),
    generic(['|']),
    section('main', 'e20', [generic(['|']), ...items(21, 4)]),
    section('dialog', 'e50', [generic(['Sure?', '|']), button('OK', 'e51')]),
    button('Done', 'e60'),
  ] as Node[]
  const lines = render(json, { text: true }).split('\n')
  expect(lines.some((line) => line.includes('"|"'))).toBe(false)
  expect(lines[5]).toBe('- main [ref=e20]:')
  const maxTokens = Math.ceil(lines.slice(0, 8).join('\n').length / 4)
  expect(render(json, { text: true, maxTokens })).toBe(
    [
      ...lines.slice(0, 8),
      "# cut: 3 more lines (3 with refs); the whole snapshot is about 100 tokens. page.getByRef('e20').ariaSnapshot() shows all of the main section. Or pass a larger maxTokens.",
      '- dialog [ref=e50]:\n  - text: Sure? |\n  - button "OK" [ref=e51]',
    ].join('\n'),
  )
})

it('tests the cut line with no section to offer, the dialog after the cut still printed', () => {
  const json = [
    section('main', 'e20', items(21, 12)),
    section('dialog', 'e50', [generic(['Sure?']), button('OK', 'e51')]),
  ]
  const lines = uncut(json)
  expect(render(json, { text: false, maxTokens: keeping(json, 4) })).toBe(
    [
      ...lines.slice(0, 4),
      '# cut: 9 more lines (9 with refs); the whole snapshot is about 200 tokens. Pass a larger maxTokens.',
      '- dialog [ref=e50]:\n  - text: Sure?\n  - button "OK" [ref=e51]',
    ].join('\n'),
  )
  expect(render(json, { text: false, maxTokens: 1_000 })).toBe(lines.join('\n'))
})

it('tests that the notice counts toward the cap', () => {
  const json = [
    ...sized(13_022),
    { role: 'alert', ref: 'e9', text: 'Expired' },
  ] as Node[]
  const notice =
    '# text left out: about 3300 tokens. Pass { text: true } to include it.'
  expect(render(json, { maxTokens: (notice.length + 1) / 4 })).toBe(
    `${notice}\n# cut: 1 more lines (1 with refs); the whole snapshot is about 100 tokens. Pass a larger maxTokens.\n- alert: Expired`,
  )
})

it('tests that a kept block is never split: the cut goes before it and it prints whole, at the left edge', () => {
  const json = [
    section('main', 'e20', [
      ...items(21, 6),
      section('alertdialog', 'e30', [
        generic(['Leave this page?']),
        button('Leave', 'e31'),
      ]),
      ...items(32, 1),
    ]),
  ]
  const lines = uncut(json)
  expect(lines[7]).toBe('  - alertdialog [ref=e30]:')
  // The limit falls on the dialog's text, so the cut moves back before its line.
  expect(render(json, { text: false, maxTokens: keeping(json, 9) })).toBe(
    [
      ...lines.slice(0, 7),
      '# cut: 1 more lines (1 with refs); the whole snapshot is about 100 tokens. Pass a larger maxTokens.',
      '- alertdialog [ref=e30]:\n  - text: Leave this page?\n  - button "Leave" [ref=e31]',
    ].join('\n'),
  )
})

it('tests that when only a kept block passes the limit, nothing is cut and the text stays as it was', () => {
  const json = [
    section('main', 'e1', [
      button('A', 'e2'),
      button('B', 'e3'),
      section('dialog', 'e4', [generic(['Sure?']), button('OK', 'e5')]),
    ]),
  ]
  const lines = uncut(json)
  expect(lines[3]).toBe('  - dialog [ref=e4]:')
  // The limit falls on the dialog's text; the dialog is the only thing past it.
  expect(render(json, { text: false, maxTokens: keeping(json, 4) })).toBe(
    lines.join('\n'),
  )
})

it('tests that a kept block inside another prints once after the cut, inside the outer one', () => {
  const json = [
    section('main', 'e20', [
      ...items(21, 12),
      section('dialog', 'e50', [
        generic(['Confirm payment']),
        {
          role: 'alert',
          children: [generic(['Card declined']), button('Retry', 'e52')],
        },
        button('OK', 'e53'),
      ]),
    ]),
  ] as Node[]
  const lines = uncut(json)
  expect(render(json, { text: false, maxTokens: keeping(json, 4) })).toBe(
    [
      ...lines.slice(0, 4),
      '# cut: 9 more lines (9 with refs); the whole snapshot is about 200 tokens. Pass a larger maxTokens.',
      '- dialog [ref=e50]:',
      '  - text: Confirm payment',
      '  - alert:',
      '    - text: Card declined',
      '    - button "Retry" [ref=e52]',
      '  - button "OK" [ref=e53]',
    ].join('\n'),
  )
})
