/**
 * The delta: its first lines, its hunks, its bounds, and the forms it compares.
 */
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import type { SnapshotBaseline } from '../../src/prelude/wire'
import { calls, type Gap, keptBy, delta as since, tree } from './model'

const link = (name: string, ref: string): AriaNodeJSON => ({
  role: 'link',
  name,
  ref,
})

const words = (sentences: number): string =>
  Array.from(
    { length: sentences },
    () => 'Each mug is thrown, glazed and fired by hand.',
  ).join(' ')

const MUGS = Array.from(
  { length: 30 },
  (_, index): AriaNodeJSON => ({ role: 'img', name: `Mug ${index + 1}` }),
)

const noted = (notes: Record<number, string>): (AriaNodeJSON | string)[] =>
  MUGS.flatMap((each, index) => {
    const note = notes[index + 1]
    return note === undefined ? [each] : [each, note]
  })

type Shop = {
  mainRef?: string
  heading?: string
  mugs?: (AriaNodeJSON | string)[]
  form?: AriaNodeJSON[]
  formAfter?: number
  footer?: string
}

function shop({
  mainRef = 'e4',
  heading = 'Mugs',
  mugs = MUGS,
  form = [{ role: 'textbox', name: 'Coupon', ref: 'e6' }],
  formAfter = mugs.length,
  footer = 'Terms',
}: Shop = {}): AriaNodeJSON[] {
  return [
    {
      role: 'main',
      ref: mainRef,
      children: [
        { role: 'heading', name: heading, level: 1 },
        ...mugs.slice(0, formAfter),
        { role: 'form', ref: 'e5', children: form },
        ...mugs.slice(formAfter),
      ],
    },
    { role: 'contentinfo', ref: 'e8', children: [link(footer, 'e9')] },
  ]
}

const small = (
  sentence = 'Two mugs left.',
  more: AriaNodeJSON[] = [],
): AriaNodeJSON[] => [
  {
    role: 'main',
    ref: 'e1',
    children: [
      sentence,
      {
        role: 'link',
        name: 'Mugs',
        ref: 'e2',
        url: 'https://shop.example/mugs',
      },
      ...more,
    ],
  },
]

const SHELF = ['Cups', 'Bowls', 'Jugs', 'Pots', 'Plates', 'Trays', 'Vases'].map(
  (name, index) => link(name, `e${index + 10}`),
)

const drawer = (links: string[]): AriaNodeJSON[] => [
  ...small(),
  {
    role: 'complementary',
    ref: 'e3',
    inert: true,
    children: links.map((name, index) => link(name, `e${index + 4}`)),
  },
]

const FOOTER: AriaNodeJSON = {
  role: 'contentinfo',
  ref: 'e30',
  children: [
    link('Home', 'e31'),
    link('Help', 'e32'),
    link('Terms', 'e33'),
    words(5),
  ],
}

const gallery = (names: string[]): AriaNodeJSON[] => [
  {
    role: 'main',
    ref: 'e1',
    children: [
      { role: 'heading', name: 'Gallery', level: 1 },
      ...names.map((name): AriaNodeJSON => ({ role: 'img', name })),
    ],
  },
  FOOTER,
]

const STOCK: Record<string, string> = {
  'Red mug': 'e2',
  'Blue mug': 'e3',
  'Tan mug': 'e4',
}

const stock = (names: string[]): AriaNodeJSON[] => [
  {
    role: 'main',
    ref: 'e1',
    children: names.flatMap((name) => [link(name, STOCK[name]), 'In stock']),
  },
  FOOTER,
]

const order = (status: string, offers: boolean): AriaNodeJSON[] => [
  {
    role: 'main',
    ref: 'e1',
    children: [
      { role: 'heading', name: 'Orders', level: 1 },
      link('Order 12, a mug and jug', 'e2'),
      link('Mugs', 'e3'),
      link('Cups', 'e4'),
      link('Bowls', 'e5'),
      `Order 12 ${status}.`,
      link('Help', 'e6'),
    ],
  },
  {
    role: 'complementary',
    ref: 'e7',
    inert: true,
    children: [
      ...(offers
        ? [{ role: 'heading', name: 'Offers', level: 2 } as const]
        : []),
      ...[
        'Sale',
        'Gifts',
        'Mugs',
        'Cups',
        'Jugs',
        'Bowls',
        'Plates',
        'Pots',
      ].map((name, index) => link(name, `e${index + 8}`)),
    ],
  },
]

const NOTES = words(91)

const notesThen = (empty: number): SnapshotBaseline => ({
  text: [
    '- main [ref=e1]:',
    `  - text: ${NOTES}`,
    '  - link "Mugs" [ref=e2]',
    '  - link "Cups" [ref=e3]',
    '  - link "Help" [ref=e4]',
    ...Array.from({ length: empty }, () => ''),
    '  - link "Jugs" [ref=e5]',
    '  - link "Pots" [ref=e6]',
    '  - link "Bowls" [ref=e7]',
  ].join('\n'),
  form: { text: 'kept', urls: false, maxTokens: 8000 },
})

export const delta: Gap = {
  snapshots: {
    // A sentence and a link with its url, which each form of the text prints differently.
    'delta/small': tree(small()),
    // The same page with a shelf of links: a whole page keeps its text, so its baseline is kept.
    'delta/shelf': tree(small(undefined, SHELF)),
    // Its text past the 12,000 units of the length rule: compared with text, given back without.
    'delta/big': tree(small(words(270), SHELF)),
    // Its text under that limit: a baseline under the notice keeps the notice, renumbered.
    'delta/small-longer': tree(small(words(16), SHELF)),
    // One more link, past a cut at 6 tokens: the cut line is not compared.
    'delta/small-more': tree(small(undefined, [link('Cups', 'e3')])),
    // An inert drawer of one link: `inert` is part of the form.
    'delta/drawer': tree(drawer(['Sale'])),
    // An inert drawer of two links: the `# not listed:` line is not compared.
    'delta/drawer-more': tree(drawer(['Sale', 'Gifts'])),
    // Pictures, short lines, so a hunk of a few lines is under half: each change below edits it.
    'delta/shop': tree(shop()),
    // A line added inside the form: the hunk names the form, three lines either side.
    'delta/coupon-error': tree(
      shop({
        form: [
          { role: 'textbox', name: 'Coupon', ref: 'e6' },
          { role: 'alert', children: ['Expired.'] },
        ],
      }),
    ),
    // The second line and the last changed: the context stops at the text's ends.
    'delta/ends': tree(shop({ heading: 'All mugs', footer: 'Rules' })),
    // Lines added with six unchanged lines between them: one hunk.
    'delta/six-apart': tree(shop({ mugs: noted({ 9: 'Sold', 15: 'Sold' }) })),
    // Lines added with seven unchanged lines between them: two hunks.
    'delta/seven-apart': tree(shop({ mugs: noted({ 9: 'Sold', 16: 'Sold' }) })),
    // The form moved up past three pictures: removed where it was, added where it is.
    'delta/form-moved': tree(shop({ formAfter: 27 })),
    // A new ref on main, a picture gone below: a hunk that only removes is named from the old text.
    'delta/removed-under-new-ref': tree(
      shop({ mainRef: 'e40', mugs: MUGS.filter((_, index) => index !== 13) }),
    ),
    // A new ref on main and a line added far below: a hunk that adds is named from the new text.
    'delta/added-under-new-ref': tree(
      shop({ mainRef: 'e40', mugs: noted({ 14: 'Sold' }) }),
    ),
    // Two pictures added near the top, the form's last line gone: found from its old-text place.
    'delta/shifted-removal': tree(
      shop({
        mugs: [
          MUGS[0],
          { role: 'img', name: 'New 1' },
          { role: 'img', name: 'New 2' },
          ...MUGS.slice(1),
        ],
      }),
    ),
    // Another page altogether: too much differs, refused before the search.
    'delta/elsewhere': tree([
      {
        role: 'main',
        ref: 'e50',
        children: [
          { role: 'heading', name: 'Sign in', level: 1 },
          { role: 'textbox', name: 'Email', ref: 'e51' },
          { role: 'button', name: 'Sign in', ref: 'e52' },
        ],
      },
    ]),
    // Pictures that print only with the page's text: the baseline of the ties.
    'delta/gallery': tree(gallery(['Red mug', 'Blue mug', 'Tan mug'])),
    // Closed shadow roots unread: that line is a fact of the tree, compared as no size line is.
    'delta/gallery-roots': tree(gallery(['Red mug', 'Blue mug', 'Tan mug']), {
      closedRoots: { sent: 1, found: 2 },
    }),
    // One picture renamed: two shortest scripts, and the removal comes first.
    'delta/gallery-renamed': tree(gallery(['Red mug', 'Grey mug', 'Tan mug'])),
    // The first picture now a copy of the second: the tail is matched first, so the first is new.
    'delta/gallery-copied': tree(gallery(['Blue mug', 'Blue mug', 'Tan mug'])),
    // Two pictures swapped: two shortest scripts, and the first one moves down.
    'delta/gallery-swapped': tree(gallery(['Blue mug', 'Red mug', 'Tan mug'])),
    // A new first picture, the second gone: a tie broken toward adding misses the shortest script.
    'delta/gallery-reordered': tree(gallery(['Tan mug', 'Red mug', 'Tan mug'])),
    // Items whose stock lines repeat: the baseline of the next.
    'delta/stock': tree(stock(['Red mug', 'Blue mug', 'Tan mug'])),
    // The middle item gone: the head is matched first, so the stock line after it goes.
    'delta/stock-removed': tree(stock(['Red mug', 'Tan mug'])),
    // An order beside an inert drawer: the baseline of the half rule.
    'delta/order': tree(order('is packed', true)),
    // One line changed: the changes are half the new text, its `# not listed:` line counted.
    'delta/order-at-half': tree(order('has shipped', true)),
    // A drawer line fewer, so the new text is one unit shorter and odd: the same changes pass half.
    'delta/order-past-half': tree(order('has shipped', false)),
    // A long note and links: the stored text has 1,000 or 1,001 empty lines, which no call writes.
    'delta/notes': tree([
      {
        role: 'main',
        ref: 'e1',
        children: [
          NOTES,
          link('Mugs', 'e2'),
          link('Cups', 'e3'),
          link('Help', 'e4'),
          link('Jugs', 'e5'),
          link('Pots', 'e6'),
          link('Bowls', 'e7'),
        ],
      },
    ]),
  },
  cases: [
    ...calls('delta/small', {
      'delta-first': since({}, 'nothing'),
      'delta-first-notext-cut': since(
        { text: false, urls: true, maxTokens: 10 },
        'nothing',
      ),
      'delta-uncompared': since({}, 'notRead'),
      'delta-uncompared-urls-cut': since(
        { urls: true, maxTokens: 10 },
        'notRead',
      ),
      'delta-unchanged': since({}, keptBy('delta/small')),
      'text-after-kept': since({ text: true }, keptBy('delta/small')),
      'notext-after-kept': since({ text: false }, keptBy('delta/small')),
      'text-after-always': since(
        { text: true },
        keptBy('delta/small', { text: true }),
      ),
      'notext-after-always': since(
        { text: false },
        keptBy('delta/small', { text: true }),
      ),
      'text-after-leftout': since({ text: true }, keptBy('delta/big')),
      'notext-after-never': since(
        { text: false },
        keptBy('delta/small', { text: false }),
      ),
      'text-after-never': since(
        { text: true },
        keptBy('delta/small', { text: false }),
      ),
      'nourls-after-kept': since({ urls: false }, keptBy('delta/small')),
      'urls-after-kept': since({ urls: true }, keptBy('delta/small')),
      'nourls-after-urls': since(
        { urls: false },
        keptBy('delta/small', { urls: true }),
      ),
      'urls-after-always': since(
        { urls: true },
        keptBy('delta/small', { text: true }),
      ),
      'urls-after-never': since(
        { urls: true },
        keptBy('delta/small', { text: false }),
      ),
      'urls-after-leftout': since({ urls: true }, keptBy('delta/big')),
      'tokens-after-kept': since({ maxTokens: 8000 }, keptBy('delta/small')),
      'tokens10-after-kept': since({ maxTokens: 10 }, keptBy('delta/small')),
      'inert-after-always-urls-20': since(
        { inert: true },
        keptBy('delta/small', { text: true, urls: true, maxTokens: 20 }),
      ),
      'noinert-after-kept': since({ inert: false }, keptBy('delta/small')),
    }),
    ...calls('delta/big', {
      'delta-after-shelf': since({}, keptBy('delta/shelf')),
    }),
    ...calls('delta/small-longer', {
      'delta-after-big': since({}, keptBy('delta/big')),
      'notext-after-big': since({ text: false }, keptBy('delta/big')),
      'delta-after-big-cut': since({}, keptBy('delta/big', { maxTokens: 20 })),
    }),
    ...calls('delta/small-more', {
      'delta-after-cut6': since({}, keptBy('delta/small', { maxTokens: 6 })),
    }),
    ...calls('delta/drawer', {
      'inert-after-default': since({ inert: true }, keptBy('delta/drawer')),
      'noinert-after-default': since({ inert: false }, keptBy('delta/drawer')),
      'delta-after-inert': since({}, keptBy('delta/drawer', { inert: true })),
      'noinert-after-inert': since(
        { inert: false },
        keptBy('delta/drawer', { inert: true }),
      ),
      'urls-after-inert': since(
        { urls: true },
        keptBy('delta/drawer', { inert: true }),
      ),
    }),
    ...calls('delta/drawer-more', {
      'delta-after-drawer': since({}, keptBy('delta/drawer')),
    }),
    ...[
      'delta/coupon-error',
      'delta/ends',
      'delta/six-apart',
      'delta/seven-apart',
      'delta/form-moved',
      'delta/removed-under-new-ref',
      'delta/added-under-new-ref',
      'delta/elsewhere',
    ].flatMap((name) =>
      calls(name, { 'delta-after-shop': since({}, keptBy('delta/shop')) }),
    ),
    ...calls('delta/shifted-removal', {
      'delta-after-coupon-error': since({}, keptBy('delta/coupon-error')),
    }),
    ...calls('delta/gallery', {
      'delta-after-roots': since({}, keptBy('delta/gallery-roots')),
    }),
    ...[
      'delta/gallery-renamed',
      'delta/gallery-copied',
      'delta/gallery-swapped',
      'delta/gallery-reordered',
    ].flatMap((name) =>
      calls(name, {
        'delta-after-gallery': since({}, keptBy('delta/gallery')),
      }),
    ),
    ...calls('delta/stock-removed', {
      'delta-after-stock': since({}, keptBy('delta/stock')),
    }),
    ...['delta/order-at-half', 'delta/order-past-half'].flatMap((name) =>
      calls(name, { 'delta-after-order': since({}, keptBy('delta/order')) }),
    ),
    ...calls('delta/notes', {
      'delta-1000-changed-at-bound': since({}, notesThen(1000)),
      'delta-1001-changed-past-bound': since({}, notesThen(1001)),
    }),
  ],
}
