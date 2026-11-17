/**
 * Fractions: `maxTokens` need not be whole, and the cap is that many tokens of four units each,
 * so 2.5 tokens is 10 units and 0.1 is 0.4. A delta compares a fractional cap exactly.
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
  tree,
} from './model'

const link = (name: string, ref: string): AriaNodeJSON => ({
  role: 'link',
  name,
  ref,
})

const SECTIONS: AriaNodeJSON[] = [
  {
    role: 'navigation',
    ref: 'e1',
    children: [link('Home', 'e2'), link('Mugs', 'e3')],
  },
  {
    role: 'main',
    ref: 'e4',
    children: [
      { role: 'heading', name: 'Mugs', level: 1 },
      'Six mugs each made to order by hand.',
      link('Blue mug', 'e5'),
    ],
  },
  {
    role: 'contentinfo',
    ref: 'e6',
    children: ['Terms', 'Help', 'Jobs', 'Press', 'Shops', 'Care'].map(
      (name, index) => link(name, `e${index + 7}`),
    ),
  },
]

export const fractions: Gap = {
  snapshots: {
    // A first line of exactly 10 units: 2.5 tokens keeps it, 2.25 does not.
    'fractions/ten': tree([
      { role: 'checkbox' },
      { role: 'button', name: 'Buy', ref: 'e1' },
    ]),
    // A whole text of 11 units: cut at 2.5 tokens, whole at 2.75.
    'fractions/eleven': tree([{ role: 'radio', name: 'a' }]),
    // A main block of 121 units: a cut at 30.25 tokens offers it, at 30 not; 75 cuts in the footer.
    'fractions/sections': tree(SECTIONS),
  },
  cases: [
    ...calls('fractions/ten', {
      'page-2.5-line-at-cap': page({ maxTokens: 2.5 }),
      'page-2.25-line-past-cap': page({ maxTokens: 2.25 }),
      'page-0.1': page({ maxTokens: 0.1 }),
      'element-2.5-line-at-cap': element({ maxTokens: 2.5 }),
      'element-0.1': element({ maxTokens: 0.1 }),
      'full-2.5-line-at-cap': full(2.5),
      'full-2.25-line-past-cap': full(2.25),
      'full-0.1': full(0.1),
      'delta-2.5-after-2.5': delta(
        { maxTokens: 2.5 },
        keptBy('fractions/ten', { maxTokens: 2.5 }),
      ),
      'delta-after-2.5': delta({}, keptBy('fractions/ten', { maxTokens: 2.5 })),
      'delta-3-after-2.5': delta(
        { maxTokens: 3 },
        keptBy('fractions/ten', { maxTokens: 2.5 }),
      ),
      'delta-next-double-after-2.5': delta(
        { maxTokens: 2.5000000000000004 },
        keptBy('fractions/ten', { maxTokens: 2.5 }),
      ),
      'delta-0.1-after-0.1': delta(
        { maxTokens: 0.1 },
        keptBy('fractions/ten', { maxTokens: 0.1 }),
      ),
    }),
    ...calls('fractions/eleven', {
      'page-2.5-text-past-cap': page({ maxTokens: 2.5 }),
      'page-2.6-text-past-cap': page({ maxTokens: 2.6 }),
      'page-2.75-text-at-cap': page({ maxTokens: 2.75 }),
      'full-2.5-text-past-cap': full(2.5),
      'full-2.75-text-at-cap': full(2.75),
    }),
    ...calls('fractions/sections', {
      'page-10': page({ maxTokens: 10 }),
      'page-30-main-past-cap': page({ maxTokens: 30 }),
      'page-30.25-main-at-cap': page({ maxTokens: 30.25 }),
      'page-75': page({ maxTokens: 75 }),
      'page-1e9': page({ maxTokens: 1e9 }),
      'element-30.25-main-at-cap': element({ maxTokens: 30.25 }),
      'full-10': full(10),
      'full-30-main-past-cap': full(30),
      'full-30.25-main-at-cap': full(30.25),
      'full-75': full(75),
      'full-1e9': full(1e9),
    }),
  ],
}
