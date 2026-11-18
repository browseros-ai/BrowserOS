/**
 * Rules of Neo's formats with no case elsewhere.
 */
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import type { Facts } from './model'
import { calls, chain, full, type Gap, page, playwright, tree } from './model'

const FALLBACK = '<p>Your browser does not support iframes.</p>'
const BACKSLASH = String.fromCharCode(92)

const button = (name: string, ref: string): AriaNodeJSON => ({
  role: 'button',
  name,
  ref,
})
const link = (name: string, ref: string): AriaNodeJSON => ({
  role: 'link',
  name,
  ref,
})
const text = (...pieces: string[]): AriaNodeJSON => ({
  role: 'generic',
  children: pieces,
})

/** A dialog over part of the view, and every other fact Neo's head lines print. */
const HEAD_FACTS: Partial<Facts> = {
  onTop: [
    {
      ref: 'e9',
      line: '# on top: dialog "Cookies" [ref=e9] over part of the view\n#   its controls: button "Accept" [ref=e10], button "Close" [ref=e11]',
      wholeView: false,
    },
  ],
  unmarked: { reason: 'timeout' },
  closedRoots: { sent: 3, found: 5 },
}

/** A page with an inert part and the dialog the on-top lines name, around `main`. */
function headPage(main: (AriaNodeJSON | string)[]): AriaNodeJSON[] {
  return [
    { role: 'main', ref: 'e1', children: main },
    {
      role: 'complementary',
      ref: 'e5',
      inert: true,
      children: ['Offers', button('Subscribe', 'e6')],
    },
    {
      role: 'dialog',
      name: 'Cookies',
      ref: 'e9',
      children: [
        'We use cookies.',
        button('Accept', 'e10'),
        button('Close', 'e11'),
      ],
    },
  ]
}

const both = {
  'page-text': page({ text: true }),
  'page-notext': page({ text: false }),
}

/** Snapshot-v1's INTERACTIVE list, each role with a word to name its node. */
const INTERACTIVE: Record<string, string> = {
  button: 'Buy',
  link: 'Home',
  textbox: 'Name',
  checkbox: 'Gift',
  radio: 'Small',
  combobox: 'Size',
  listbox: 'Color',
  menuitem: 'Open',
  menuitemcheckbox: 'Bold',
  menuitemradio: 'Left',
  option: 'Red',
  searchbox: 'Find',
  slider: 'Volume',
  spinbutton: 'Count',
  switch: 'Dark',
  tab: 'Specs',
  treeitem: 'Docs',
}

/** Snapshot-v1's SECTION list. */
const SECTION = [
  'banner',
  'navigation',
  'main',
  'complementary',
  'contentinfo',
  'search',
  'form',
  'region',
  'article',
  'iframe',
]

export const rules: Gap = {
  snapshots: {
    // An iframe's fallback text: Playwright's format prints it and not the frame; Neo's formats
    // print the frame and never that text, which is no label and hides no name.
    'rules/iframe-fallback': tree([
      {
        role: 'main',
        ref: 'e1',
        children: [
          {
            role: 'iframe',
            ref: 'e2',
            text: FALLBACK,
            children: [
              { role: 'textbox', name: 'Card number', ref: 'f1e1' },
              button('Pay', 'f1e2'),
            ],
          },
          { role: 'iframe', ref: 'e3', text: FALLBACK },
          {
            role: 'generic',
            ref: 'e4',
            cursor: 'pointer',
            children: [{ role: 'iframe', ref: 'e5', text: FALLBACK }],
          },
          {
            role: 'cell',
            name: 'Card',
            children: [{ role: 'iframe', ref: 'e6', text: FALLBACK }],
          },
        ],
      },
    ]),
    // Every head line in one text. The page sits under 44 nested images whose long names print
    // only with text, over 12,000 units, so the notice is there too.
    'rules/heads': chain(
      {
        depth: 45,
        link: {
          role: 'img',
          name: 'Seats are held for ten minutes, then go back on sale. '
            .repeat(5)
            .trimEnd(),
        },
        leaf: {
          role: 'generic',
          children: headPage([
            { role: 'heading', name: 'Tickets', level: 1, ref: 'e2' },
            button('Reserve', 'e3'),
          ]),
        },
      },
      HEAD_FACTS,
    ),
    // Playwright's format prints none of Neo's head lines.
    'rules/heads-playwright': tree(
      headPage(['Choose a seat and pay.', button('Reserve', 'e3')]),
      HEAD_FACTS,
    ),
    // `mode: 'full'` counts a line as one with a ref when its page text says `[ref=`.
    'rules/ref-text': tree([
      {
        role: 'main',
        ref: 'e1',
        children: [
          button('Reserve', 'e2'),
          { role: 'paragraph', children: ['Show [ref=e7] at the desk.'] },
          button('Cancel', 'e3'),
        ],
      },
    ]),
    // A kept block prints its text in every form: one piece in the leaf form, more as a block.
    'rules/kept-text': tree([
      { role: 'alert', ref: 'e1', text: 'Card declined.' },
      {
        role: 'status',
        ref: 'e2',
        children: [text('Saved'), text('2 minutes ago')],
      },
      {
        role: 'dialog',
        name: 'Remove item?',
        ref: 'e3',
        children: [
          'Remove "Blue mug" from your cart?',
          button('Remove', 'e4'),
          button('Keep', 'e5'),
        ],
      },
      {
        role: 'alertdialog',
        ref: 'e6',
        children: [
          { role: 'paragraph', children: ['Session ends in 1 minute.'] },
        ],
      },
    ]),
    // A dialog's line keeps its ref; an alert's or a status's only when clickable, with its marker.
    'rules/kept-refs': tree([
      { role: 'dialog', ref: 'e1', text: 'Uploading…' },
      { role: 'alert', ref: 'e2', text: 'Not clickable' },
      { role: 'status', ref: 'e3', cursor: 'pointer', text: 'Undo' },
      {
        role: 'alert',
        ref: 'e4',
        cursor: 'pointer',
        children: ['Card declined', button('Retry', 'e5')],
      },
    ]),
    // A kept block with nothing to print keeps its line only when named, separators being
    // nothing; one that goes lets the text around it join.
    'rules/kept-empty': tree([
      text('Before'),
      { role: 'status', ref: 'e1' },
      text('after'),
      { role: 'alertdialog', name: 'Session ended', ref: 'e2' },
      { role: 'alert', name: 'Notice', ref: 'e3', text: '|' },
      { role: 'status', ref: 'e4', children: [text('—')] },
    ]),
    // A section prints its ref and indents its content; empty it goes and the text around it
    // joins, an iframe stays, and one without a ref prints none.
    'rules/sections': tree([
      {
        role: 'navigation',
        name: 'Primary',
        ref: 'e1',
        children: ['Menu', link('Home', 'e2')],
      },
      { role: 'region', ref: 'e3', children: ['Only text'] },
      { role: 'iframe', ref: 'e4' },
      {
        role: 'form',
        children: [{ role: 'textbox', name: 'Email', ref: 'e5' }],
      },
      text('Before'),
      { role: 'search', ref: 'e6', children: [{ role: 'img' }] },
      text('after'),
    ]),
    // A heading prints no ref, its own text when nameless, and its links on lines of their own;
    // a clickable one keeps its ref and marker.
    'rules/headings': tree([
      {
        role: 'heading',
        name: 'Results',
        level: 2,
        ref: 'e1',
        children: [link('Results', 'e2')],
      },
      {
        role: 'heading',
        level: 3,
        ref: 'e3',
        children: ['Step 2', link('edit', 'e4')],
      },
      {
        role: 'heading',
        name: 'Offers',
        level: 2,
        ref: 'e5',
        cursor: 'pointer',
      },
    ]),
    // An actionable node is one line with its ref, value and props; its nested controls follow
    // at its level; a ref without a role or a pointer to act on is no control.
    'rules/controls': tree([
      {
        role: 'textbox',
        name: 'Email',
        ref: 'e1',
        text: 'ana@mail.test',
        placeholder: 'you@mail.test',
      },
      { role: 'textbox', name: 'City', ref: 'e2', text: 'City' },
      { role: 'button', name: 'Go', ref: 'e3', cursor: 'pointer' },
      {
        role: 'generic',
        ref: 'e4',
        cursor: 'pointer',
        children: ['Add to cart'],
      },
      { role: 'generic', ref: 'e5', children: ['Plain wrapper'] },
      {
        role: 'listitem',
        ref: 'e6',
        cursor: 'pointer',
        children: ['Blue mug', button('Remove', 'e7')],
      },
      {
        role: 'combobox',
        name: 'Theme',
        ref: 'e8',
        children: [text('Dark')],
      },
      {
        role: 'link',
        name: 'Docs',
        ref: 'e9',
        url: 'https://docs.example/',
      },
    ]),
    // A field without a ref still prints its state and value, in both forms.
    'rules/fields': tree([
      { role: 'checkbox', name: 'Gift wrap', checked: true },
      { role: 'checkbox', name: 'All items', checked: 'mixed' },
      { role: 'textbox', name: 'Note', text: 'Leave at door' },
      {
        role: 'textbox',
        name: 'Name',
        children: [{ role: 'paragraph', children: ['Ana'] }],
      },
      { role: 'slider', name: 'Volume', text: '40' },
    ]),
    // A named image prints with text on only and with no ref; a nameless one prints nothing; a
    // clickable one is a control.
    'rules/images': tree([
      { role: 'img', name: 'Blue mug', ref: 'e1' },
      { role: 'img', ref: 'e2' },
      { role: 'img', name: 'Zoom', ref: 'e3', cursor: 'pointer' },
      { role: 'img', name: 'Chart', children: ['Sales rose in May'] },
    ]),
    // A node's name is its text when nothing under it prints any; a text field, a heading, a
    // ref-less field and a child that prints text each keep the name out.
    'rules/name-as-content': tree([
      {
        role: 'row',
        children: [
          { role: 'cell', name: '$7.49' },
          {
            role: 'cell',
            name: 'Blue mug',
            children: [link('Blue mug', 'e1')],
          },
        ],
      },
      {
        role: 'listitem',
        name: 'Ships today',
        children: [
          '',
          { role: 'generic', name: '' },
          { role: 'generic', children: [] },
        ],
      },
      { role: 'heading', name: 'Specs', level: 3 },
      {
        role: 'listitem',
        name: 'Deep',
        children: [text('Deep text')],
      },
      { role: 'checkbox', name: 'Insured' },
      { role: 'group', name: 'Note', text: 'Fragile' },
      button('Next', 'e2'),
      {
        role: 'columnheader',
        name: 'Full name',
        children: [{ role: 'generic', ref: 'e3', cursor: 'pointer' }],
      },
    ]),
    // A nameless control's label is the text under it outside nested controls: an image's alt
    // text counts; a nested control's name, a ref-less field, a heading's name and an inert part
    // (unless asked for) do not.
    'rules/own-text': tree([
      {
        role: 'link',
        ref: 'e1',
        url: 'https://pkg.example/acme',
        children: [{ role: 'img', name: 'version 2.1' }],
      },
      {
        role: 'generic',
        ref: 'e2',
        cursor: 'pointer',
        children: [
          'Blue mug',
          button('Remove', 'e3'),
          { role: 'checkbox', name: 'Gift wrap' },
          { role: 'heading', name: 'Mugs', level: 4 },
          { role: 'generic', inert: true, children: ['Sold out'] },
          { role: 'text', text: 'two left' },
        ],
      },
    ]),
    // A text line of separators alone is dropped, judged on the joined line: a split price
    // stays, and `#` and `*` are not separators.
    'rules/separators': tree([
      text('|'),
      link('hide', 'e1'),
      text(
        '·',
        ' – ',
        '»',
        '{ }',
        BACKSLASH,
        '‐',
        '/',
        '( )',
        ':',
        ';',
        ',',
        '.',
        '[ ]',
        '< >',
        '—',
        '•',
        '‧',
        '∙',
        '›',
        '«',
        '‹',
      ),
      link('next', 'e2'),
      text('$', '7', '.', '49'),
      button('Add', 'e3'),
      text('#', '|'),
      button('More', 'e4'),
      text('*'),
    ]),
    // A block holding only separator lines prints nothing: a section goes and the text around it
    // joins; an iframe keeps its line; a price split across a section's pieces stays.
    'rules/separators-block': tree([
      text('Before'),
      {
        role: 'navigation',
        ref: 'e1',
        children: [text('|'), text('·')],
      },
      text('after'),
      { role: 'iframe', ref: 'e2', children: ['/'] },
      { role: 'region', ref: 'e3', children: ['$', text('5')] },
    ]),
    // Separator lines dropped at the end move the block ends back with them: under a cut, the
    // open section offered and the kept block printed are whole and nothing more.
    'rules/separators-ends': tree([
      text('|'),
      {
        role: 'banner',
        ref: 'e7',
        children: [link('Home', 'e8'), link('Cart', 'e9')],
      },
      {
        role: 'main',
        ref: 'e1',
        children: [
          text('·'),
          button('A', 'e2'),
          text('|'),
          button('B', 'e3'),
          button('C', 'e4'),
        ],
      },
      { role: 'dialog', ref: 'e5', children: ['Saved.', button('OK', 'e6')] },
    ]),
    // Text pieces at one level join into one line with a space, quoted once as a whole; a line
    // or a level between them keeps them apart.
    'rules/joining': tree([
      text('3', 'items'),
      button('Buy', 'e1'),
      text('Rated', '#1'),
      { role: 'region', ref: 'e2', children: ['Ships', 'tomorrow'] },
      text('Free returns'),
    ]),
    // Every interactive role, with a ref and without, over the same value: with a ref each is a
    // control; the field roles print their value, with or without a ref; the others print it as
    // page text without one, and nothing of it with one.
    'rules/roles-interactive': tree(
      Object.entries(INTERACTIVE).flatMap(([role, name], index) => [
        {
          role: role as AriaNodeJSON['role'],
          name,
          ref: `e${index + 1}`,
          children: ['Picked'],
        },
        { role: role as AriaNodeJSON['role'], name, children: ['Picked'] },
      ]),
    ),
    // Every section role prints its line with its ref and indents its content.
    'rules/roles-section': tree(
      SECTION.map((role, index) => ({
        role: role as AriaNodeJSON['role'],
        ref: `e${2 * index + 1}`,
        children: [link('Home', `e${2 * index + 2}`)],
      })),
    ),
    // A dropped separator line moves a kept block's end back: under a cut inside the block, the
    // block prints whole and the line after it is not part of it.
    'rules/separators-kept-end': tree([
      text('|'),
      { role: 'dialog', ref: 'e1', children: ['Saved.', button('OK', 'e2')] },
      button('Undo', 'e3'),
      button('Close', 'e4'),
    ]),
    // A nameless link with no words and no url prints no label; with a url, the url is its label.
    'rules/link-no-url': tree([
      { role: 'link', ref: 'e1' },
      { role: 'link', ref: 'e2', url: 'https://shop.example/cart' },
    ]),
    // Empty text pieces add no space to a control's own text: a label, a value, a heading's text.
    'rules/own-text-empty': tree([
      {
        role: 'generic',
        ref: 'e1',
        cursor: 'pointer',
        children: ['', 'Add', '', 'to cart', ''],
      },
      {
        role: 'textbox',
        name: 'Name',
        ref: 'e2',
        children: ['', { role: 'text', text: '' }, 'Ana'],
      },
      { role: 'heading', level: 2, children: ['', 'Step 2'] },
    ]),
    // An empty named kept block is still a kept block: past a cut it prints after the cut line.
    'rules/kept-empty-cut': tree([
      button('Undo', 'e1'),
      button('Close', 'e2'),
      { role: 'alertdialog', name: 'Session ended', ref: 'e3' },
      button('Help', 'e4'),
    ]),
    // A whole-view layer's content prints first, with its text in every form, and its last text
    // never joins the page's first.
    'rules/front-layer': tree(
      [
        { role: 'paragraph', children: ['Welcome back.'] },
        button('Continue', 'e2'),
        {
          role: 'generic',
          ref: 'e9',
          onTop: 'e9',
          children: ['Read this first.'],
        },
      ],
      {
        onTop: [
          {
            ref: 'e9',
            line: '# on top: generic [ref=e9], says "Read this first." over the whole view',
            wholeView: true,
          },
        ],
      },
    ),
  },
  cases: [
    ...calls('rules/iframe-fallback', {
      playwright: playwright(),
      full: full(),
      ...both,
    }),
    ...calls('rules/heads', {
      'page-max1': page({ maxTokens: 1 }),
      'page-max25': page({ maxTokens: 25 }),
      full1: full(1),
    }),
    ...calls('rules/heads-playwright', { playwright: playwright() }),
    ...calls('rules/ref-text', {
      full12: full(12),
      'page-max12': page({ maxTokens: 12 }),
    }),
    ...calls('rules/kept-text', both),
    ...calls('rules/kept-refs', { 'page-notext': page({ text: false }) }),
    ...calls('rules/kept-empty', both),
    ...calls('rules/sections', both),
    ...calls('rules/headings', both),
    ...calls('rules/controls', {
      'page-notext': page({ text: false }),
      'page-urls': page({ urls: true }),
    }),
    ...calls('rules/fields', both),
    ...calls('rules/images', both),
    ...calls('rules/name-as-content', both),
    ...calls('rules/own-text', {
      'page-notext': page({ text: false }),
      'page-inert': page({ inert: true }),
    }),
    ...calls('rules/separators', { full: full(), ...both }),
    ...calls('rules/separators-block', both),
    ...calls('rules/separators-ends', {
      'page-max25': page({ maxTokens: 25 }),
    }),
    ...calls('rules/joining', { playwright: playwright(), ...both }),
    ...calls('rules/front-layer', { full: full(), ...both }),
    ...calls('rules/roles-interactive', both),
    ...calls('rules/roles-section', { 'page-notext': page({ text: false }) }),
    ...calls('rules/separators-kept-end', {
      'page-max10': page({ maxTokens: 10 }),
    }),
    ...calls('rules/kept-empty-cut', { 'page-max6': page({ maxTokens: 6 }) }),
    ...calls('rules/link-no-url', {
      'page-notext': page({ text: false }),
      'page-notext-urls': page({ text: false, urls: true }),
    }),
    ...calls('rules/own-text-empty', both),
  ],
}
