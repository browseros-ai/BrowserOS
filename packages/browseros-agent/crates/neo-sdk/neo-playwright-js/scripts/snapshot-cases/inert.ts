/**
 * The inert rule: what Rust marks inert is left out of Neo's format and counted (`# not listed:`), listed under `{ inert: true }`, and headed `# inert:` in `mode: 'full'`.
 */
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import {
  calls,
  delta,
  element,
  full,
  type Gap,
  keptBy,
  page,
  playwright,
  tree,
} from './model'

const marked = (node: AriaNodeJSON): AriaNodeJSON => ({ ...node, inert: true })

const tickets = (): AriaNodeJSON => ({
  role: 'main',
  ref: 'e1',
  children: [
    { role: 'heading', name: 'Tickets', level: 1, ref: 'e2' },
    'Choose a seat.',
    ...['Reserve', 'Pay'].map(
      (name, index): AriaNodeJSON => ({
        role: 'button',
        name,
        ref: `e${index + 3}`,
      }),
    ),
  ],
})

const offers = (): AriaNodeJSON => ({
  role: 'complementary',
  ref: 'e7',
  children: ['Offers', { role: 'button', name: 'Join', ref: 'e8' }],
})

const paragraph = (text: string): AriaNodeJSON =>
  marked({ role: 'paragraph', children: [text] })

const GO: AriaNodeJSON = { role: 'button', name: 'Go', ref: 'e1' }

export const inert: Gap = {
  snapshots: {
    // No mark: the page the deltas start from and come back to.
    'inert/page': tree([tickets(), offers()]),
    // A root at the top level: left out and counted, listed under { inert: true }, headed in full mode.
    'inert/side': tree([tickets(), marked(offers())]),
    // A modal dialog open: the page behind it inert, two roots back to back, the dialog not.
    'inert/dialog-open': tree([
      marked(tickets()),
      marked(offers()),
      {
        role: 'dialog',
        name: 'Leave?',
        ref: 'e9',
        children: [
          'Your seats are not saved.',
          { role: 'button', name: 'Leave', ref: 'e10' },
        ],
      },
    ]),
    // Roots nested in live blocks: a region, and part of a heading's and of a nameless button's own text.
    'inert/nested': tree([
      {
        role: 'main',
        ref: 'e1',
        children: [
          {
            role: 'heading',
            level: 1,
            ref: 'e2',
            children: ['Seats', paragraph('(sold out)')],
          },
          {
            role: 'button',
            ref: 'e3',
            children: ['Book', paragraph('now')],
          },
          marked({
            role: 'region',
            name: 'Offers',
            ref: 'e4',
            children: [{ role: 'button', name: 'Join', ref: 'e5' }],
          }),
        ],
      },
    ]),
    // A frame not read inside an inert part: its `# partial:` line goes with it, and comes back under { inert: true }.
    'inert/partial-inside': tree(
      [
        GO,
        marked({
          role: 'region',
          name: 'Card',
          ref: 'e2',
          children: [{ role: 'iframe', ref: 'e3' }],
        }),
      ],
      {
        partial: [
          {
            ref: 'e3',
            frame: 'https://pay.example/card',
            reason: 'timeout after 2000 ms',
          },
        ],
      },
    ),
    // Inside kept blocks: counted with their text under { text: false }; an alert left with one text prints as a leaf.
    'inert/in-kept-block': tree([
      {
        role: 'dialog',
        name: 'Leave?',
        ref: 'e1',
        children: [
          'Your seats are not saved.',
          marked({ role: 'paragraph', ref: 'e2', text: 'Saved at noon.' }),
          { role: 'button', name: 'Leave', ref: 'e3' },
        ],
      },
      {
        role: 'alert',
        ref: 'e4',
        children: [
          'Upload failed.',
          marked({ role: 'button', name: 'Retry', ref: 'e5' }),
        ],
      },
    ]),
    // Inert parts that print nothing: `# not listed: 0 lines`; full mode counts the lines it prints for them.
    'inert/prints-nothing': tree([
      GO,
      marked({ role: 'generic' }),
      paragraph('·'),
    ]),
    // Roots back to back: one omission, their texts joined into one line.
    'inert/back-to-back': tree([paragraph('One'), paragraph('Two'), GO]),
    // A printed line between two roots: two omissions, two lines.
    'inert/after-control': tree([paragraph('One'), GO, paragraph('Two')]),
    // A text piece joined onto the line the first root followed: two omissions, two lines.
    'inert/after-text': tree([
      { role: 'text', text: 'Before' },
      paragraph('One'),
      { role: 'text', text: 'between' },
      paragraph('Two'),
      GO,
    ]),
    // A region that printed nothing between two roots: both follow the same line, one omission, one line.
    'inert/after-vanished': tree([
      GO,
      paragraph('One'),
      { role: 'region', ref: 'e2' },
      paragraph('Two'),
    ]),
    // An inert part holding an iframe with fallback text: both counts take the frame's content,
    // never that text.
    'inert/iframe-fallback': tree([
      GO,
      marked({
        role: 'region',
        name: 'Card',
        ref: 'e2',
        children: [
          {
            role: 'iframe',
            ref: 'e3',
            text: '<p>Your browser does not support iframes.</p>',
            children: [
              { role: 'textbox', name: 'Card number', ref: 'f1e1' },
              { role: 'button', name: 'Pay', ref: 'f1e2' },
            ],
          },
        ],
      }),
    ]),
    // Every head line in order: on top, refs missing, not listed, the text notice, closed roots.
    'inert/heads': tree(
      [
        {
          role: 'paragraph',
          children: [Array.from({ length: 2450 }, () => 'seat').join(' ')],
        },
        marked(offers()),
        {
          role: 'generic',
          ref: 'e9',
          onTop: 'e9',
          children: [
            'Sale ends today.',
            { role: 'button', name: 'Shop', ref: 'e10' },
          ],
        },
        GO,
      ],
      {
        onTop: [
          {
            ref: 'e9',
            line: '# on top: generic [ref=e9] over the whole view\n#   its controls: button "Shop" [ref=e10]',
            wholeView: true,
          },
        ],
        unmarked: { reason: 'timeout' },
        closedRoots: { sent: 1, found: 2 },
      },
    ),
  },
  cases: [
    ...calls('inert/side', {
      full: full(),
      element: element(),
      'element-notext': element({ text: false }),
      'element-inert': element({ inert: true }),
      'element-inert-notext': element({ inert: true, text: false }),
      'delta-inert-kept': delta({}, keptBy('inert/side', { inert: true })),
      'delta-inert-switched-off': delta(
        { inert: false },
        keptBy('inert/side', { inert: true }),
      ),
    }),
    ...calls('inert/dialog-open', {
      'delta-after-page': delta({}, keptBy('inert/page')),
    }),
    ...calls('inert/page', {
      'delta-after-dialog-open': delta({}, keptBy('inert/dialog-open')),
    }),
    ...calls('inert/nested', {
      element: element(),
      'element-inert': element({ inert: true }),
    }),
    ...calls('inert/partial-inside', {
      full: full(),
      element: element(),
      'element-inert': element({ inert: true }),
    }),
    ...calls('inert/in-kept-block', {
      'element-notext': element({ text: false }),
      'element-inert-notext': element({ inert: true, text: false }),
    }),
    ...calls('inert/prints-nothing', {
      playwright: playwright(),
      full: full(),
      element: element(),
    }),
    ...calls('inert/back-to-back', {
      element: element(),
      'element-notext': element({ text: false }),
      'element-inert': element({ inert: true }),
    }),
    ...calls('inert/after-control', { element: element() }),
    ...calls('inert/after-text', {
      element: element(),
      'element-inert': element({ inert: true }),
    }),
    ...calls('inert/after-vanished', { element: element() }),
    ...calls('inert/iframe-fallback', { full: full(), element: element() }),
    ...calls('inert/heads', {
      page: page(),
      'page-tiny': page({ maxTokens: 1 }),
    }),
  ],
}
