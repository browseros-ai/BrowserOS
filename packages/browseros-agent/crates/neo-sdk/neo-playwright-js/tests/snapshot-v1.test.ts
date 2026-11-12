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
    generic(['|']),
  ]
  expect(both(json)).toEqual([
    '- text: "Price #1 in Toothpaste"\n- button "Buy" [ref=e2]\n- text: "|"',
    '- button "Buy" [ref=e2]',
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

it('tests the cut line offering the outermost open section that fits the cap', () => {
  const json = [
    section('banner', 'e1', items(10, 6)),
    section('main', 'e20', items(21, 4)),
    section('contentinfo', 'e40', items(41, 2)),
  ]
  const lines = uncut(json)
  // The banner and two of main's buttons fit; main (5 lines) fits the cap on its own.
  expect(render(json, { text: false, maxTokens: keeping(json, 10) })).toBe(
    [
      ...lines.slice(0, 10),
      "# cut: 5 more lines (5 with refs); the whole snapshot is about 200 tokens. page.getByRef('e20').ariaSnapshot() shows all of main. Or pass a larger maxTokens.",
    ].join('\n'),
  )
})

it('tests the cut line offering the next section when the open one is larger than the cap', () => {
  const json = [
    section('main', 'e20', items(21, 12)),
    section('contentinfo', 'e40', items(41, 2), 'Footer'),
  ]
  const lines = uncut(json)
  expect(render(json, { text: false, maxTokens: keeping(json, 4) })).toBe(
    [
      ...lines.slice(0, 4),
      '# cut: 12 more lines (12 with refs); the whole snapshot is about 200 tokens. ' +
        'page.getByRef(\'e40\').ariaSnapshot() shows contentinfo "Footer", which is not shown above. Or pass a larger maxTokens.',
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
