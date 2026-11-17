/**
 * Playwright's own format: default-mode trees, unmarked trees, frames.
 */
import {
  calls,
  element,
  full,
  type Gap,
  page,
  tree,
  playwright as yaml,
} from './model'

const units = (count: number) => 'a'.repeat(count)
const smile = '\u{1F600}'

// A tree as `mode: 'default'` reads it, with no ref and no cursor.
const defaultPage = tree([
  {
    role: 'banner',
    children: [
      { role: 'link', name: 'Home', url: 'https://shop.example/' },
      {
        role: 'navigation',
        name: 'Main',
        children: [
          {
            role: 'list',
            children: [
              {
                role: 'listitem',
                children: [
                  {
                    role: 'link',
                    name: 'Mugs',
                    url: 'https://shop.example/mugs',
                  },
                ],
              },
              {
                role: 'listitem',
                children: [
                  {
                    role: 'link',
                    name: 'Cups',
                    url: 'https://shop.example/cups',
                  },
                ],
              },
            ],
          },
        ],
      },
    ],
  },
  {
    role: 'main',
    children: [
      { role: 'heading', name: 'Mugs', level: 1 },
      { role: 'paragraph', children: ['Made to order.', 'Ships in a week.'] },
      { role: 'img', name: 'A blue mug' },
      {
        role: 'textbox',
        name: 'Search',
        placeholder: 'Find a mug',
      },
      { role: 'checkbox', name: 'In stock only', checked: true },
      { role: 'button', name: 'Add to cart', disabled: true },
      { role: 'combobox', name: 'Size', expanded: true, text: 'Large' },
    ],
  },
  { role: 'contentinfo', children: ['Shop of mugs.'] },
])

// A node's /url and /placeholder come before its text or its children; a node with a text prints no children.
const propsTextChildren = tree([
  {
    role: 'link',
    name: 'Docs',
    ref: 'e1',
    url: 'https://docs.example/',
    children: [{ role: 'img', name: 'Logo' }, 'Read the docs'],
  },
  {
    role: 'link',
    ref: 'e2',
    url: 'https://docs.example/start',
    text: 'Start here',
  },
  {
    role: 'link',
    ref: 'e3',
    url: 'https://docs.example/hidden',
    text: 'Shown',
    children: ['Never shown', { role: 'button', name: 'Hidden', ref: 'e4' }],
  },
  {
    role: 'textbox',
    name: 'Name',
    ref: 'e5',
    placeholder: 'Your name',
    text: 'Ada',
  },
  {
    role: 'textbox',
    name: 'Note',
    ref: 'e6',
    url: 'https://docs.example/note',
    placeholder: 'A note',
    children: ['Draft'],
  },
  {
    role: 'paragraph',
    ref: 'e7',
    text: 'Own text',
    children: ['Child text', { role: 'link', name: 'Gone', ref: 'e8' }],
  },
])

// An empty text prints `- text: ""`, as a child, as a text node with or without its text, and as a node's own text.
const emptyText = tree([
  {
    role: 'paragraph',
    ref: 'e1',
    children: ['', { role: 'text', text: '' }, { role: 'text' }, 'After'],
  },
  { role: 'paragraph', ref: 'e2', text: '' },
  { role: 'link', ref: 'e3', url: 'https://empty.example/', text: '' },
])

// A name of up to 900 units prints and a longer one is dropped; an astral character is two units.
const nameLength = tree([
  { role: 'button', name: units(899), ref: 'e1' },
  { role: 'button', name: units(900), ref: 'e2' },
  { role: 'button', name: units(901), ref: 'e3' },
  { role: 'button', name: units(902), ref: 'e4' },
  { role: 'button', name: `${units(898)}${smile}`, ref: 'e5' },
  { role: 'button', name: `${units(899)}${smile}`, ref: 'e6' },
])

// Playwright's text keeps an iframe's fallback text and prints no frame under it; Neo's formats print the frame.
const iframeFallback = tree([
  {
    role: 'main',
    ref: 'e1',
    children: [
      {
        role: 'iframe',
        ref: 'e2',
        text: 'Your browser does not support frames.',
        children: [
          { role: 'textbox', name: 'Card number', ref: 'f1e1' },
          { role: 'button', name: 'Pay', ref: 'f1e2' },
        ],
      },
      { role: 'iframe', ref: 'e3', text: 'No frames here.' },
    ],
  },
])

// Two frames unread under one iframe print in reverse in Playwright's text and `mode: 'full'`, in order in Neo's.
const partialTwice = tree(
  [
    {
      role: 'main',
      ref: 'e1',
      children: [
        { role: 'iframe', ref: 'e2' },
        {
          role: 'iframe',
          ref: 'e3',
          children: [{ role: 'button', name: 'Go', ref: 'f1e1' }],
        },
        { role: 'button', name: 'Back', ref: 'e4' },
      ],
    },
  ],
  {
    partial: [
      {
        ref: 'e2',
        frame: 'https://first.example/',
        reason: 'timeout after 2000 ms',
      },
      { ref: 'e3', frame: 'https://third.example/', reason: 'target closed' },
      { ref: 'e2', frame: 'https://second.example/', reason: 'frame detached' },
    ],
  },
)

// Frames unread whose iframes sit under a fallback text Playwright's text prints instead: placed under a line that says the ref, else last.
const partialHidden = tree(
  [
    {
      role: 'main',
      ref: 'e1',
      children: [
        {
          role: 'iframe',
          ref: 'e2',
          text: 'Frame [ref=e3] did not load.',
          children: [
            { role: 'iframe', ref: 'e3' },
            { role: 'iframe', ref: 'e4' },
          ],
        },
        { role: 'button', name: 'Back', ref: 'e5' },
      ],
    },
  ],
  {
    partial: [
      { ref: 'e3', frame: 'https://said.example/', reason: 'frame detached' },
      { ref: 'e4', frame: 'https://unsaid.example/', reason: 'target closed' },
    ],
  },
)

// Neo's facts and marks beside a tree: Playwright's text prints none of them and keeps document order.
const neoFacts = tree(
  [
    {
      role: 'main',
      ref: 'e1',
      inert: true,
      children: [
        { role: 'heading', name: 'Tickets', level: 1 },
        { role: 'button', name: 'Reserve', ref: 'e2' },
      ],
    },
    {
      role: 'dialog',
      name: 'Sign in',
      ref: 'e9',
      onTop: 'e9',
      children: [
        'Sign in to go on.',
        { role: 'button', name: 'Close', ref: 'e10' },
      ],
    },
  ],
  {
    onTop: [
      {
        ref: 'e9',
        line: '# on top: dialog "Sign in" [ref=e9] over the whole view\n#   its controls: button "Close" [ref=e10]',
        wholeView: true,
      },
    ],
    unmarked: { reason: 'timeout' },
    closedRoots: { found: 5, sent: 3 },
  },
)

export const playwright: Gap = {
  snapshots: {
    'playwright/default-page': defaultPage,
    'playwright/props-text-children': propsTextChildren,
    'playwright/empty-text': emptyText,
    'playwright/name-length': nameLength,
    'playwright/iframe-fallback': iframeFallback,
    'playwright/partial-twice': partialTwice,
    'playwright/partial-hidden': partialHidden,
    'playwright/neo-facts': neoFacts,
  },
  cases: [
    ...calls('playwright/default-page', { playwright: yaml() }),
    ...calls('playwright/props-text-children', {
      playwright: yaml(),
      full: full(),
      'page-urls': page({ urls: true }),
    }),
    ...calls('playwright/empty-text', {
      playwright: yaml(),
      full: full(),
      'page-text-urls': page({ text: true, urls: true }),
    }),
    ...calls('playwright/name-length', {
      playwright: yaml(),
      element: element(),
    }),
    ...calls('playwright/iframe-fallback', {
      playwright: yaml(),
      full: full(),
      page: page(),
    }),
    ...calls('playwright/partial-twice', {
      playwright: yaml(),
      full: full(),
      page: page(),
    }),
    ...calls('playwright/partial-hidden', {
      playwright: yaml(),
      full: full(),
      page: page(),
    }),
    ...calls('playwright/neo-facts', {
      playwright: yaml(),
      full: full(),
      page: page(),
    }),
  ],
}
