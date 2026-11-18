/**
 * The limits, met exactly and passed by one: a line that ran is not a boundary that was met.
 */
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import {
  type CallDef,
  calls,
  chain,
  delta,
  element,
  full,
  type Gap,
  page,
  tree,
} from './model'

/** One astral character: two UTF-16 units. */
const EMOJI = String.fromCodePoint(0x1f600)

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
const section = (
  role: string,
  ref: string,
  children: AriaNodeJSON[],
  name?: string,
): AriaNodeJSON => ({
  role: role as AriaNodeJSON['role'],
  ref,
  ...(name === undefined ? {} : { name }),
  children,
})
/** `units` UTF-16 units of text, an emoji in the middle when there is room for one. */
function filler(units: number): string {
  if (units < 2) return 'a'.repeat(units)
  const before = Math.floor((units - 2) / 2)
  return `${'a'.repeat(before)}${EMOJI}${'a'.repeat(units - 2 - before)}`
}

/**
 * 44 nested regions, each line `- region "<209 units>":` indented two units a level, holding one
 * paragraph: 11,756 units of text form before the paragraph's own text.
 */
function longPage(paragraph: number) {
  return chain({
    depth: 45,
    link: {
      role: 'region',
      name: 'Part of a long page. '.repeat(10).trimEnd(),
    },
    leaf: { role: 'paragraph', children: [filler(paragraph)] },
  })
}

const banner = section('banner', 'e1', [link('Home', 'e2'), link('Cart', 'e3')])

/** A call through both formats the cap reaches, at one limit. */
function capped(maxTokens: number): Record<string, CallDef> {
  return {
    [`page-max${maxTokens}`]: page({ maxTokens }),
    [`full${maxTokens}`]: full(maxTokens),
  }
}

/** A stored text under the notice: a delta renders the page again in that form. */
const UNDER_NOTICE = {
  text: '- button "Go" [ref=e1]',
  form: { text: 'leftOut', urls: false, maxTokens: 8000 },
} as const

/** The same stored text, its form listing the inert parts. */
const UNDER_NOTICE_INERT = {
  text: '- button "Go" [ref=e1]',
  form: { text: 'leftOut', urls: false, maxTokens: 8000, inert: true },
} as const

/** A dialog of one line, the page's first, then buttons: the same lines in both capped formats. */
function savedFirst(buttons: string[]): AriaNodeJSON[] {
  return [
    { role: 'dialog', name: 'Saved', ref: 'e1', text: 'Changes saved.' },
    ...buttons.map((name, index) => button(name, `e${index + 2}`)),
  ]
}

const deals = (ref: number): AriaNodeJSON =>
  section(
    'region',
    `e${ref}`,
    [button('C', `e${ref + 1}`), button('D', `e${ref + 2}`)],
    'Deals',
  )

export const boundaries: Gap = {
  snapshots: {
    // A text form of exactly 12,000 units, an emoji in it: the text is kept. A locator's call,
    // so the 12,000 units are not written twice as a pending baseline.
    'boundaries/text-12000': longPage(244),
    // A text form of 12,001 units, an emoji in it: the text is left out under the notice.
    'boundaries/text-12001': longPage(245),
    // The limit (52 units) at the end of the second line: that line is taken.
    'boundaries/cap-line-at-limit': tree([
      button('One', 'e1'),
      button(`Go ${EMOJI} up`, 'e2'),
      button('Three', 'e3'),
    ]),
    // The second line ends one unit past the limit: it is not taken.
    'boundaries/cap-line-past-limit': tree([
      button('One', 'e1'),
      button(`Go ${EMOJI} top`, 'e2'),
      button('Three', 'e3'),
    ]),
    // A whole text of exactly the limit: nothing is cut.
    'boundaries/cap-whole-at-limit': tree([
      button('One', 'e1'),
      button(`Go ${EMOJI} up`, 'e2'),
    ]),
    // A whole text one unit past the limit: it is cut.
    'boundaries/cap-whole-past-limit': tree([
      button('One', 'e1'),
      button(`Go ${EMOJI} top`, 'e2'),
    ]),
    // A cut text of 1,000 units: 2.5 hundred tokens, a half, which Math.round takes up to 300.
    'boundaries/about-half': tree([
      button('Go', 'e1'),
      { role: 'paragraph', children: [filler(969)] },
    ]),
    // A cut text of 999 units: just under the half, 200.
    'boundaries/about-below-half': tree([
      button('Go', 'e1'),
      { role: 'paragraph', children: [filler(968)] },
    ]),
    // A cut text of 199 units rounds to no tokens: the estimate says at least 100.
    'boundaries/about-floor': tree([
      button('Go', 'e1'),
      { role: 'paragraph', children: [filler(168)] },
    ]),
    // 1,000 units of text left out, counted by walking the page again with its text: 300.
    'boundaries/notice-half': tree([
      button('Go', 'e1'),
      { role: 'paragraph', children: [filler(991)] },
    ]),
    // No text left out at all, under a stored form that leaves text out: still about 100.
    'boundaries/notice-floor': tree([button('Go', 'e1')]),
    // The cut falls inside a section whose own lines fit the limit: that section is offered.
    'boundaries/offer-open': tree([
      banner,
      section('main', 'e4', [
        button('One', 'e5'),
        button('Two', 'e6'),
        button('Six', 'e7'),
      ]),
    ]),
    // Two open sections that fit: the outermost is offered.
    'boundaries/offer-outermost': tree([
      banner,
      section('main', 'e4', [
        section(
          'region',
          'e5',
          [button('One', 'e6'), button('Two', 'e7')],
          'Deals',
        ),
      ]),
    ]),
    // The open section larger than the limit is never offered; the one inside it that fits is.
    'boundaries/offer-inner': tree([
      banner,
      section('main', 'e4', [
        section(
          'region',
          'e5',
          [button('One', 'e6'), button('Two', 'e7')],
          'Deals',
        ),
        button('Six', 'e8'),
        button('Ten', 'e9'),
      ]),
    ]),
    // No open section fits: the next section after the cut is offered, however large.
    'boundaries/offer-later': tree([
      section('main', 'e1', [
        button('One', 'e2'),
        button('Two', 'e3'),
        button('Six', 'e4'),
        button('Ten', 'e5'),
      ]),
      section(
        'navigation',
        'e6',
        [link('About us', 'e7'), link('Contact', 'e8')],
        'Footer',
      ),
    ]),
    // No open section fits and none follows: only a larger maxTokens is offered.
    'boundaries/offer-none': tree([
      section('main', 'e1', [
        button('One', 'e2'),
        button('Two', 'e3'),
        button('Six', 'e4'),
        button('Ten', 'e5'),
      ]),
    ]),
    // An open section whose own lines are exactly the limit (100 units): offered.
    'boundaries/offer-fits-exactly': tree([
      banner,
      section('main', 'e4', [
        button('One', 'e5'),
        button('Two', 'e6'),
        button(`Up ${EMOJI} top`, 'e7'),
      ]),
    ]),
    // An open section one unit past the limit: not offered.
    'boundaries/offer-one-past': tree([
      banner,
      section('main', 'e4', [
        button('One', 'e5'),
        button('Two', 'e6'),
        button(`Up ${EMOJI} tops`, 'e7'),
      ]),
    ]),
    // The limit inside a dialog: the cut goes before it, and it prints whole after the cut line
    // at the left edge, the alert inside it once.
    'boundaries/kept-not-split': tree([
      section('main', 'e1', [
        button('One', 'e2'),
        button('Two', 'e3'),
        {
          role: 'dialog',
          name: 'Confirm',
          ref: 'e4',
          children: [
            'Pay now?',
            { role: 'alert', ref: 'e5', text: 'Card declined.' },
            button('Yes', 'e6'),
            button('No', 'e7'),
          ],
        },
        button('Three', 'e8'),
      ]),
    ]),
    // A kept block that is the first line and holds the limit: the cut goes before it, so the
    // text opens with the cut line.
    'boundaries/kept-first-holds-limit': tree([
      {
        role: 'dialog',
        name: 'Confirm',
        ref: 'e1',
        children: ['Pay now?', button('Yes', 'e2'), button('No', 'e3')],
      },
      button('One', 'e4'),
      button('Two', 'e5'),
    ]),
    // A kept block that ends exactly at the cut stays above the cut line.
    'boundaries/kept-ends-at-cut': tree(savedFirst(['One', 'Two', 'Three'])),
    // A kept block wholly above the cut, lines after it taken too, stays where it is.
    'boundaries/kept-above-cut': tree(
      savedFirst(['One', 'Two', 'Three', 'Four']),
    ),
    // A section that starts exactly at the cut and fits the limit is not open there: it is
    // offered as not shown above, not as all of it.
    'boundaries/offer-starts-at-cut': tree([
      button('Add to cart', 'e1'),
      button('Add to list', 'e2'),
      deals(3),
    ]),
    // A section whose own lines are the limit (74 units) and end exactly at the cut is not open
    // there: it is not offered.
    'boundaries/offer-ends-at-cut': tree([
      deals(1),
      button('Add to cart', 'e4'),
      button('Add to list', 'e5'),
    ]),
    // A section with a ref and nothing under it is never offered; the next section is. Neo's
    // format prints no line for it at all.
    'boundaries/offer-empty-section': tree([
      button('One', 'e1'),
      button('Two', 'e2'),
      section('region', 'e3', [], 'Ads'),
      section(
        'navigation',
        'e4',
        [link('About us', 'e5'), link('Contact', 'e6')],
        'Footer',
      ),
    ]),
    // 1,000 units of text left out when the inert part counts, 400 when it does not: under a
    // stored form that lists the inert parts, its text is in the number.
    'boundaries/notice-inert': tree([
      button('Go', 'e1'),
      { role: 'paragraph', children: [filler(391)] },
      { role: 'paragraph', inert: true, children: [filler(599)] },
    ]),
    // Every line past the limit is in a kept block: nothing is cut, the text stays as it was.
    'boundaries/kept-only-past': tree([
      button('One', 'e1'),
      button('Two', 'e2'),
      {
        role: 'dialog',
        name: 'Saved',
        ref: 'e3',
        children: ['Your changes are saved.', button('OK', 'e4')],
      },
    ]),
    // Text alone past 12,000 units: no line prints, and the notice is longer than the limit.
    'boundaries/notice-alone': tree([
      { role: 'paragraph', children: ['w '.repeat(6000)] },
    ]),
    // The same, with closed roots not all read: their pinned line follows the notice.
    'boundaries/notice-alone-roots': tree(
      [{ role: 'paragraph', children: ['w '.repeat(6000)] }],
      { closedRoots: { sent: 1, found: 2 } },
    ),
  },
  cases: [
    ...calls('boundaries/text-12000', { element: element() }),
    ...calls('boundaries/text-12001', { page: page() }),
    ...calls('boundaries/cap-line-at-limit', capped(13)),
    ...calls('boundaries/cap-line-past-limit', capped(13)),
    ...calls('boundaries/cap-whole-at-limit', {
      'page-max13': page({ maxTokens: 13 }),
    }),
    ...calls('boundaries/cap-whole-past-limit', {
      'page-max13': page({ maxTokens: 13 }),
    }),
    ...calls('boundaries/about-half', {
      'page-text-max10': page({ text: true, maxTokens: 10 }),
    }),
    ...calls('boundaries/about-below-half', {
      'page-text-max10': page({ text: true, maxTokens: 10 }),
    }),
    ...calls('boundaries/about-floor', {
      'page-text-max10': page({ text: true, maxTokens: 10 }),
    }),
    ...calls('boundaries/notice-half', {
      'delta-under-notice': delta({}, UNDER_NOTICE),
    }),
    ...calls('boundaries/notice-floor', {
      'delta-under-notice': delta({}, UNDER_NOTICE),
    }),
    ...calls('boundaries/offer-open', capped(25)),
    ...calls('boundaries/offer-outermost', {
      'page-max29': page({ maxTokens: 29 }),
    }),
    ...calls('boundaries/offer-inner', capped(29)),
    ...calls('boundaries/offer-later', capped(15)),
    ...calls('boundaries/offer-none', {
      'page-max15': page({ maxTokens: 15 }),
    }),
    ...calls('boundaries/offer-fits-exactly', capped(25)),
    ...calls('boundaries/offer-one-past', {
      'page-max25': page({ maxTokens: 25 }),
    }),
    ...calls('boundaries/kept-not-split', capped(30)),
    ...calls('boundaries/kept-only-past', capped(20)),
    ...calls('boundaries/kept-first-holds-limit', capped(10)),
    ...calls('boundaries/kept-ends-at-cut', capped(12)),
    ...calls('boundaries/kept-above-cut', capped(24)),
    ...calls('boundaries/offer-starts-at-cut', capped(20)),
    ...calls('boundaries/offer-ends-at-cut', capped(18.5)),
    ...calls('boundaries/offer-empty-section', capped(6)),
    ...calls('boundaries/notice-inert', {
      'delta-under-notice-inert': delta({}, UNDER_NOTICE_INERT),
      'delta-under-notice': delta({}, UNDER_NOTICE),
    }),
    ...calls('boundaries/notice-alone', {
      'page-max1': page({ maxTokens: 1 }),
      'page-max17': page({ maxTokens: 17 }),
    }),
    ...calls('boundaries/notice-alone-roots', {
      'page-max1': page({ maxTokens: 1 }),
      'page-max17': page({ maxTokens: 17 }),
    }),
  ],
}
