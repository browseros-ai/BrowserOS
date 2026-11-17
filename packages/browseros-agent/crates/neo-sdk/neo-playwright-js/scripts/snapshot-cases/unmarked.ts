/**
 * The `# refs missing:` line: the check for clickable elements did not finish on the root frame.
 */
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import {
  calls,
  delta,
  element,
  type Facts,
  full,
  type Gap,
  keptBy,
  page,
  playwright,
  type Snapshot,
  tree,
} from './model'

type Layer = NonNullable<Facts['onTop']>[number]

const buttons = (...names: string[]): AriaNodeJSON[] =>
  names.map((name, index) => ({ role: 'button', name, ref: `e${index + 1}` }))

const ACTIONS = ['Save', 'Delete', 'Share', 'Close']

const SHOP: Layer = {
  ref: 'e9',
  line: '# on top: generic [ref=e9] over the whole view\n#   its controls: button "Shop" [ref=e10]',
  wholeView: true,
}

const heads = (onTop: Layer[]): Snapshot =>
  tree(
    [
      { role: 'button', name: 'Save', ref: 'e1' },
      { role: 'button', name: 'Old', ref: 'e2', inert: true },
      {
        role: 'generic',
        ref: 'e9',
        onTop: 'e9',
        children: [{ role: 'button', name: 'Shop', ref: 'e10' }],
      },
    ],
    {
      onTop,
      unmarked: { reason: 'timeout' },
      closedRoots: { sent: 1, found: 2 },
    },
  )

const card = (marked: boolean): AriaNodeJSON[] => [
  {
    role: 'main',
    ref: 'e1',
    children: [
      { role: 'button', name: 'Reserve', ref: 'e3' },
      marked
        ? { role: 'generic', name: 'Card', ref: 'e5', cursor: 'pointer' }
        : { role: 'generic', name: 'Card' },
    ],
  },
]

const REASONS = [
  'frame detached',
  'refused',
  'document not found',
  'browser error',
  'node resolve failed',
  'page shape',
]

const named = (reason: string) =>
  `unmarked/reason-${reason.replaceAll(' ', '-')}`

export const unmarked: Gap = {
  snapshots: {
    // The line, outside the cap in Neo's format and full mode, never in Playwright's format.
    'unmarked/page': tree(buttons(...ACTIONS), {
      unmarked: { reason: 'timeout' },
    }),
    // The same page with its check finished: no line, and no change in a delta.
    'unmarked/checked': tree(buttons(...ACTIONS)),
    // One button renamed with the check unfinished: the line after the status, never in a hunk.
    'unmarked/changed': tree(buttons('Save', 'Remove', 'Share', 'Close'), {
      unmarked: { reason: 'timeout' },
    }),
    ...Object.fromEntries(
      REASONS.map((reason) => [
        // Each other word the call log gives for why the check did not finish.
        named(reason),
        tree(buttons('Go'), { unmarked: { reason } }),
      ]),
    ),
    // The head lines in order: on top, refs missing, not listed (inert in full mode), closed roots.
    'unmarked/heads': heads([SHOP]),
    // The same under one more layer that is gone now: refs missing comes after `# no longer on top:`.
    'unmarked/heads-two-layers': heads([
      SHOP,
      {
        ref: 'e20',
        line: '# on top: dialog "Cookies" [ref=e20] over part of the view',
        wholeView: false,
      },
    ]),
    // The half rule counts the line: a card that lost its ref is listed after the marked one.
    'unmarked/unmarked-card': tree(card(false), {
      unmarked: { reason: 'timeout' },
    }),
    // Without the line the same one-line change back is more than half, and the page comes back whole.
    'unmarked/marked-card': tree(card(true)),
  },
  cases: [
    ...calls('unmarked/page', {
      playwright: playwright(),
      full20: full(20),
      element20: element({ maxTokens: 20 }),
      'delta-first': delta({}, 'nothing'),
      'delta-uncompared': delta({}, 'notRead'),
      'delta-after-checked': delta({}, keptBy('unmarked/checked')),
      'delta-too-much': delta({ urls: true }, keptBy('unmarked/page')),
    }),
    ...calls('unmarked/checked', {
      'delta-after-page': delta({}, keptBy('unmarked/page')),
    }),
    ...calls('unmarked/changed', {
      'delta-after-page': delta({}, keptBy('unmarked/page')),
    }),
    ...REASONS.flatMap((reason) =>
      calls(named(reason), { element: element() }),
    ),
    ...calls('unmarked/heads', {
      full: full(),
      page: page(),
      'delta-after-two-layers': delta({}, keptBy('unmarked/heads-two-layers')),
    }),
    ...calls('unmarked/unmarked-card', {
      'delta-after-marked-card': delta({}, keptBy('unmarked/marked-card')),
    }),
    ...calls('unmarked/marked-card', {
      'delta-after-unmarked-card': delta({}, keptBy('unmarked/unmarked-card')),
    }),
  ],
}
