/**
 * Closed shadow roots not all read: the pinned `# partial:` head line, outside the cap.
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

/** One astral character: two UTF-16 units. */
const EMOJI = String.fromCodePoint(0x1f600)

/** `units` UTF-16 units of text, an emoji in the middle. */
function filler(units: number): string {
  const before = Math.floor((units - 2) / 2)
  return `${'a'.repeat(before)}${EMOJI}${'a'.repeat(units - 2 - before)}`
}

/** A button over a paragraph of `units`, its line read with the closed roots not all read. */
const longer = (units: number) =>
  tree(
    [
      { role: 'button', name: 'Go', ref: 'e1' },
      { role: 'paragraph', children: [filler(units)] },
    ],
    { closedRoots: { sent: 1, found: 2 } },
  )

const rows = (): AriaNodeJSON[] => [
  {
    role: 'main',
    ref: 'e1',
    children: 'ABCDEFGHIJKLMN'.split('').map(
      (row, index): AriaNodeJSON => ({
        role: 'button',
        name: row,
        ref: `e${index + 2}`,
      }),
    ),
  },
]

export const closedRoots: Gap = {
  snapshots: {
    // The line opens Neo's format and full mode, never Playwright's; it is not measured, so a body at the limit stays whole.
    'closed-roots/small': tree([{ role: 'button', name: 'Go', ref: 'e1' }], {
      closedRoots: { sent: 0, found: 2 },
    }),
    // A cut text of 999 units beside the line: the cut line's size leaves the line out, so it
    // says 200 tokens, just under the half that would round up to 300.
    'closed-roots/size-below-half': longer(968),
    // The same in full mode, which prints the paragraph as a line above its text.
    'closed-roots/size-below-half-full': longer(953),
    // The line on a longer page, the baseline of a delta.
    'closed-roots/rows': tree(rows(), { closedRoots: { sent: 3, found: 5 } }),
    // One more root read: the delta lists the line as a change of the page.
    'closed-roots/rows-more-read': tree(rows(), {
      closedRoots: { sent: 4, found: 5 },
    }),
  },
  cases: [
    ...calls('closed-roots/small', {
      playwright: playwright(),
      full: full(),
      element: element(),
      page: page(),
      'page-at-limit': page({ maxTokens: 5.5 }),
      'page-one-over': page({ maxTokens: 5.25 }),
      'full-at-limit': full(5.5),
      'full-one-over': full(5.25),
      'element-tiny': element({ maxTokens: 1 }),
      'delta-first-tiny': delta({ maxTokens: 1 }, 'nothing'),
      'delta-uncompared': delta({}, 'notRead'),
      'delta-too-much': delta({ urls: true }, keptBy('closed-roots/small')),
    }),
    ...calls('closed-roots/size-below-half', {
      'page-text-max10': page({ text: true, maxTokens: 10 }),
    }),
    ...calls('closed-roots/size-below-half-full', { full10: full(10) }),
    ...calls('closed-roots/rows-more-read', {
      'delta-after-rows': delta({}, keptBy('closed-roots/rows')),
    }),
  ],
}
