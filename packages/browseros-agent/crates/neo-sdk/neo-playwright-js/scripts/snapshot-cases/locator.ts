/**
 * A locator's snapshot: the tree rooted at one element, in Neo's format and `mode: 'full'`.
 */
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import { calls, element, full, type Gap, tree } from './model'

const footer: AriaNodeJSON = {
  role: 'navigation',
  name: 'Footer',
  ref: 'e7',
  children: [
    'Find us',
    { role: 'link', name: 'Contact', ref: 'e8', url: 'https://shop.example/c' },
    { role: 'link', name: 'Jobs', ref: 'e9', url: 'https://shop.example/j' },
  ],
}

export const locator: Gap = {
  snapshots: {
    // A control as the root: its one line, its own text as its label and its url on request.
    'locator/control': tree([
      {
        role: 'link',
        ref: 'e5',
        url: 'https://shop.example/more',
        cursor: 'pointer',
        children: [{ role: 'img', name: 'Arrow' }, 'Read more'],
      },
    ]),
    // A section as the root: its line with its ref, and its content indented under it.
    'locator/section': tree([footer]),
    // A text node as the root: its one text line, and nothing at all without text.
    'locator/text': tree([{ role: 'text', text: 'Open: 9 to 5' }]),
    // A kept block as the root: its text in every form, and never cut, whatever the limit.
    'locator/kept': tree([
      {
        role: 'dialog',
        name: 'Leave?',
        ref: 'e3',
        children: [
          'Your cart will be kept.',
          { role: 'button', name: 'Stay', ref: 'e4' },
          { role: 'button', name: 'Leave', ref: 'e5' },
        ],
      },
    ]),
    // A wrapper as the root prints no line: its content moves up to the left edge.
    'locator/wrapper': tree([
      {
        role: 'generic',
        ref: 'e2',
        children: [
          { role: 'heading', name: 'Cart', level: 2, ref: 'e3' },
          'Two items',
          { role: 'button', name: 'Pay', ref: 'e4' },
        ],
      },
    ]),
    // An inert root: the whole answer is the line saying what was not listed.
    'locator/inert': tree([{ ...footer, inert: true }]),
    // A root whose own line does not fit: the cut line offers the root itself.
    'locator/offered': tree([footer]),
  },
  cases: [
    ...calls('locator/control', {
      element: element(),
      'element-notext': element({ text: false }),
      'element-urls': element({ urls: true }),
      full: full(),
    }),
    ...calls('locator/section', {
      element: element(),
      'element-notext': element({ text: false }),
      full: full(),
    }),
    ...calls('locator/text', {
      element: element(),
      'element-notext': element({ text: false }),
      full: full(),
    }),
    ...calls('locator/kept', {
      'element-notext': element({ text: false }),
      'element-max1': element({ maxTokens: 1 }),
      full1: full(1),
    }),
    ...calls('locator/wrapper', {
      element: element(),
      'element-notext': element({ text: false }),
      full: full(),
    }),
    ...calls('locator/inert', {
      element: element(),
      'element-inert': element({ inert: true }),
      full: full(),
    }),
    ...calls('locator/offered', {
      'element-max3': element({ maxTokens: 3 }),
      full3: full(3),
    }),
  ],
}
