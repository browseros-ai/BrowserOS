/**
 * The attributes of a node's key, the numbers they print, and absent against empty.
 */
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import {
  calls,
  delta,
  element,
  everyFormat,
  type Facts,
  full,
  type Gap,
  keptBy,
  page,
  playwright,
  tree,
} from './model'

// Each attribute with each value Playwright's types allow, alone on a node.
const every = tree([
  { role: 'checkbox', name: 'Checked', ref: 'e1', checked: true },
  { role: 'checkbox', name: 'Mixed', ref: 'e2', checked: 'mixed' },
  { role: 'checkbox', name: 'Field', checked: true },
  { role: 'button', name: 'Disabled', ref: 'e3', disabled: true },
  { role: 'button', name: 'Expanded', ref: 'e4', expanded: true },
  { role: 'button', name: 'Active', ref: 'e5', active: true },
  { role: 'textbox', name: 'Invalid', ref: 'e6', invalid: true },
  { role: 'textbox', name: 'Grammar', ref: 'e7', invalid: 'grammar' },
  { role: 'textbox', name: 'Spelling', ref: 'e8', invalid: 'spelling' },
  { role: 'heading', name: 'Level', level: 3 },
  { role: 'button', name: 'Pressed', ref: 'e9', pressed: true },
  { role: 'button', name: 'Half', ref: 'e10', pressed: 'mixed' },
  { role: 'tab', name: 'Selected', ref: 'e11', selected: true },
  { role: 'button', name: 'Hidden', ref: 'e12', ariaHidden: true },
  { role: 'generic', name: 'Card', ref: 'e13', cursor: 'pointer' },
  { role: 'button', name: 'Pointer', ref: 'e14', cursor: 'pointer' },
  { role: 'generic', name: 'Loose', cursor: 'pointer' },
  {
    role: 'button',
    name: 'Box',
    ref: 'e15',
    box: { x: 10, y: 20, width: 30, height: 40 },
  },
])

// Every attribute on one node prints in one fixed order, the ref and its cursor after the states and the box last.
const order = tree([
  {
    role: 'treeitem',
    name: 'All',
    ref: 'e1',
    cursor: 'pointer',
    box: { x: 1, y: 2, width: 3, height: 4 },
    ariaHidden: true,
    selected: true,
    pressed: 'mixed',
    level: 2,
    invalid: 'grammar',
    active: true,
    expanded: true,
    disabled: true,
    checked: 'mixed',
  },
  {
    role: 'generic',
    name: 'Trues',
    ref: 'e2',
    cursor: 'pointer',
    pressed: true,
    invalid: true,
    checked: true,
  },
])

// A box and a level print as JavaScript prints a number: plain below 1e21, an exponent from it, the shortest digits, a tie to the even digit.
const numbers = tree([
  {
    role: 'button',
    name: 'Whole',
    ref: 'e1',
    box: { x: 0, y: 7, width: 1280, height: 720 },
  },
  {
    role: 'button',
    name: 'Large',
    ref: 'e2',
    box: { x: 2.5, y: 1e20, width: 1e21, height: 1.7976931348623157e308 },
  },
  {
    role: 'button',
    name: 'Small',
    ref: 'e3',
    box: { x: 0.000001, y: 1e-7, width: 5e-324, height: 0.1 + 0.2 },
  },
  {
    role: 'button',
    name: 'Tie',
    ref: 'e4',
    box: {
      x: -(167581363823776 + 1 / 8),
      y: -1,
      width: -0.5,
      height: 123.456,
    },
  },
  { role: 'heading', name: 'Zero', level: 0 },
  { role: 'heading', name: 'One', level: 1 },
  { role: 'heading', name: 'Half', level: 2.5 },
  { role: 'heading', name: 'Huge', level: 1e21 },
  { role: 'heading', name: 'Below', level: -1 },
])

// Absent against empty: Playwright's printer tests a ref, a name and a level for truth, Neo's format tests a ref and a name for presence.
const absentEmpty = tree([
  {
    role: 'main',
    ref: 'e1',
    children: [
      { role: 'button', name: 'Empty ref', ref: '' },
      { role: 'button', name: 'No ref' },
      { role: 'generic', name: 'Empty card', ref: '', cursor: 'pointer' },
      { role: 'generic', name: 'No card', cursor: 'pointer' },
      { role: 'link', name: '', ref: 'e2', url: 'https://empty.example/' },
      { role: 'link', ref: 'e3', url: 'https://none.example/' },
      { role: 'textbox', name: '', ref: 'e4', text: '' },
      { role: 'textbox', ref: 'e5', text: '' },
      { role: 'heading', name: '', level: 0, children: ['Untitled'] },
      { role: 'heading', children: ['Plain'] },
      { role: 'dialog', name: '', ref: 'e6' },
      { role: 'dialog', ref: 'e7' },
      { role: 'dialog', name: 'Named', ref: 'e8' },
      { role: 'region', name: '', ref: '', children: ['Inside'] },
      { role: 'region', children: ['Beside'] },
    ],
  },
])

// A section with an empty ref is offered by the cut line, and a line with an empty ref counts as one with a ref in Neo's format, not in `mode: 'full'`.
const emptyRefCut = tree([
  {
    role: 'main',
    ref: 'e1',
    children: [
      { role: 'heading', name: 'Rows', level: 1 },
      {
        role: 'region',
        name: '',
        ref: '',
        children: [
          { role: 'button', name: 'A', ref: 'e2' },
          { role: 'button', name: 'B', ref: '' },
          { role: 'button', name: 'C', ref: '' },
        ],
      },
      { role: 'button', name: 'D', ref: '' },
      { role: 'button', name: 'E', ref: 'e3' },
    ],
  },
])

// A frame unread under an empty ref: Playwright's text places it under the first line whose node has that ref, Neo's under the iframe.
const emptyRefPartial = tree(
  [
    {
      role: 'main',
      ref: 'e1',
      children: [
        { role: 'button', name: 'Before', ref: '' },
        { role: 'iframe', ref: '' },
        { role: 'button', name: 'After', ref: 'e2' },
      ],
    },
  ],
  {
    partial: [
      {
        ref: '',
        frame: 'https://frame.example/',
        reason: 'timeout after 2000 ms',
      },
    ],
  },
)

type Admitted = Partial<
  Record<
    | 'checked'
    | 'pressed'
    | 'invalid'
    | 'disabled'
    | 'expanded'
    | 'active'
    | 'inert'
    | 'selected'
    | 'ariaHidden',
    false
  >
> & { cursor?: string }

// Values no page writes, which Rust's typed tree admits: upstream's type forbids them.
const admitted = (node: AriaNodeJSON, values: Admitted): AriaNodeJSON =>
  ({ ...node, ...values }) as AriaNodeJSON

const FALSES: Admitted = {
  checked: false,
  pressed: false,
  invalid: false,
  disabled: false,
  expanded: false,
  active: false,
  inert: false,
  selected: false,
  ariaHidden: false,
}

/**
 * Each value where a test for presence would print differently: the key fields on a control and
 * a section, cursors on a section, a kept block, controls and nodes that are none, `inert` inside
 * a nameless control's text and on a whole-view layer, `selected` on a long select's options. `set` gives a node its
 * values: `admitted`, or nothing for the twin.
 */
function admittedPage(
  set: (node: AriaNodeJSON, values: Admitted) => AriaNodeJSON,
): AriaNodeJSON[] {
  return [
    set(
      {
        role: 'region',
        name: 'Terms',
        ref: 'e1',
        children: [
          'Read these first.',
          set({ role: 'checkbox', name: 'Agree', ref: 'e2' }, FALSES),
          set(
            { role: 'button', name: 'Send', ref: 'e3' },
            { cursor: 'default' },
          ),
          set(
            { role: 'generic', name: 'Card', ref: 'e4' },
            { cursor: 'default' },
          ),
          set({ role: 'generic', name: 'Tile', ref: 'e5' }, { cursor: '' }),
          set(
            { role: 'generic', name: 'Row', ref: 'e6' },
            { cursor: 'POINTER' },
          ),
          set({ role: 'generic', name: 'Note' }, { cursor: 'default' }),
          {
            role: 'generic',
            ref: 'e7',
            cursor: 'pointer',
            children: [
              'Buy',
              set({ role: 'generic', children: ['now'] }, { inert: false }),
            ],
          },
          {
            role: 'combobox',
            name: 'Size',
            ref: 'e8',
            children: Array.from({ length: 13 }, (_, index) =>
              index === 3
                ? { role: 'option', name: `Size ${index + 1}`, selected: true }
                : set(
                    { role: 'option', name: `Size ${index + 1}` },
                    { selected: false },
                  ),
            ),
          },
        ],
      },
      { ...FALSES, cursor: 'default' },
    ),
    set(
      {
        role: 'dialog',
        name: 'Offer',
        ref: 'e20',
        onTop: 'e20',
        children: [
          'Save ten percent.',
          { role: 'button', name: 'Close', ref: 'e21' },
        ],
      },
      { inert: false, cursor: '' },
    ),
  ]
}

const OFFER: Partial<Facts> = {
  onTop: [
    {
      ref: 'e20',
      line: '# on top: dialog "Offer" [ref=e20] over the whole view\n#   its controls: button "Close" [ref=e21]',
      wholeView: true,
    },
  ],
}

const { 'delta-uncompared': _uncompared, ...formats } = everyFormat()

export const attributes: Gap = {
  snapshots: {
    'attributes/every': every,
    'attributes/order': order,
    'attributes/numbers': numbers,
    'attributes/absent-empty': absentEmpty,
    'attributes/empty-ref-cut': emptyRefCut,
    'attributes/empty-ref-partial': emptyRefPartial,
    // An explicit false in each true-or-absent field, and a cursor that is not 'pointer': each
    // prints as if the field were absent, in every format.
    'attributes/false-values': tree(admittedPage(admitted), OFFER),
    // The same page without those fields: every text is the one above.
    'attributes/false-values-twin': tree(
      admittedPage((node) => node),
      OFFER,
    ),
  },
  cases: [
    ...calls('attributes/every', {
      playwright: playwright(),
      element: element(),
    }),
    ...calls('attributes/order', {
      playwright: playwright(),
      element: element(),
    }),
    ...calls('attributes/numbers', {
      playwright: playwright(),
      element: element(),
    }),
    ...calls('attributes/absent-empty', {
      playwright: playwright(),
      full: full(),
      page: page(),
      'page-notext': page({ text: false }),
      'delta-first': delta({}, 'nothing'),
    }),
    ...calls('attributes/empty-ref-cut', {
      element20: element({ maxTokens: 20 }),
      full20: full(20),
    }),
    ...calls('attributes/false-values', {
      ...formats,
      'delta-after-twin': delta({}, keptBy('attributes/false-values-twin')),
    }),
    ...calls('attributes/false-values-twin', formats),
    ...calls('attributes/empty-ref-partial', {
      playwright: playwright(),
      full: full(),
      page: page(),
      'delta-first': delta({}, 'nothing'),
    }),
  ],
}
