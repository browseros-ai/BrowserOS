/**
 * The clip: a nameless control's own text or a link's url label, cut at 80 units without text.
 */
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import { calls, full, type Gap, page, tree } from './model'

/** One astral character: two UTF-16 units. */
const EMOJI = String.fromCodePoint(0x1f600)
const SITE = 'https://shop.example/'

/** A nameless clickable whose label is its pieces, joined with one space. */
function clickable(ref: string, pieces: string[]): AriaNodeJSON {
  return { role: 'generic', ref, cursor: 'pointer', children: pieces }
}

/** A nameless link that says nothing but its url. */
function bare(ref: string, url: string): AriaNodeJSON {
  return { role: 'link', ref, url }
}

const both = {
  'page-text': page({ text: true }),
  'page-notext': page({ text: false }),
}

export const clip: Gap = {
  snapshots: {
    // Both labels clipped without text, whole with it, and the url's own line whole with urls.
    'clip/labels': tree([
      clickable('e1', [
        'Free shipping on every order over forty dollars,',
        'and free returns for thirty days',
      ]),
      bare(
        'e2',
        `${SITE}search?q=blue+mugs+with+handles&sort=price&page=2&view=grid&size=large`,
      ),
    ]),
    // A control's own text field and a field's value are never clipped, however long.
    'clip/not-labels': tree([
      {
        role: 'button',
        ref: 'e1',
        text: 'Free shipping on every order over forty dollars, and free returns for thirty days',
      },
      {
        role: 'textbox',
        ref: 'e2',
        children: [
          {
            role: 'paragraph',
            children: [
              'Leave the parcel at the side door, behind the blue gate, then ring the bell twice and wait.',
            ],
          },
        ],
      },
    ]),
    // Labels of exactly 80 units: not clipped.
    'clip/at-80': tree([
      clickable('e1', ['a'.repeat(39), 'b'.repeat(40)]),
      bare('e2', SITE + 'x'.repeat(80 - SITE.length)),
    ]),
    // Labels of 81 units: clipped to 80 and marked.
    'clip/past-80': tree([
      clickable('e1', ['a'.repeat(39), 'b'.repeat(41)]),
      bare('e2', SITE + 'x'.repeat(81 - SITE.length)),
    ]),
    // An emoji on units 79 and 80 of a two-piece label ends the clip whole.
    'clip/emoji-ends-at-80': tree([
      clickable('e1', ['a'.repeat(40), `${'b'.repeat(37)}${EMOJI}cc`]),
    ]),
    // An emoji on units 80 and 81: the clip backs up to 79 and never ends in half of it.
    'clip/emoji-across-80': tree([
      clickable('e1', ['a'.repeat(40), `${'b'.repeat(38)}${EMOJI}c`]),
    ]),
    // The first and the last character outside the first 65,536 (U+10000, U+10FFFF) on units 80
    // and 81: the clip backs up to 79 for both ends of the high half's range.
    'clip/astral-ends-across-80': tree([
      clickable('e1', [
        'a'.repeat(40),
        `${'b'.repeat(38)}${String.fromCodePoint(0x10000)}c`,
      ]),
      clickable('e2', [
        'a'.repeat(40),
        `${'b'.repeat(38)}${String.fromCodePoint(0x10ffff)}c`,
      ]),
    ]),
    // The clip trims what it ends on by JavaScript's whitespace: the joining space and U+FEFF,
    // never U+0085.
    'clip/trim': tree([
      clickable('e1', ['a'.repeat(79), 'bc']),
      clickable('e2', [`${'a'.repeat(79)}${String.fromCodePoint(0xfeff)}bc`]),
      clickable('e3', [`${'a'.repeat(79)}${String.fromCodePoint(0x85)}bc`]),
    ]),
  },
  cases: [
    ...calls('clip/labels', {
      full: full(),
      ...both,
      'page-notext-urls': page({ text: false, urls: true }),
    }),
    ...calls('clip/not-labels', { 'page-notext': page({ text: false }) }),
    ...calls('clip/at-80', { 'page-notext': page({ text: false }) }),
    ...calls('clip/past-80', { 'page-notext': page({ text: false }) }),
    ...calls('clip/emoji-ends-at-80', { 'page-notext': page({ text: false }) }),
    ...calls('clip/emoji-across-80', { 'page-notext': page({ text: false }) }),
    ...calls('clip/astral-ends-across-80', {
      'page-notext': page({ text: false }),
    }),
    ...calls('clip/trim', { 'page-notext': page({ text: false }) }),
  ],
}
