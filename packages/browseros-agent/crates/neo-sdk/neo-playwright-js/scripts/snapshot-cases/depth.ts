/**
 * Depth: trees at the bound Rust decodes and, as cases of the type, far past it.
 */
import {
  calls,
  chain,
  delta,
  element,
  full,
  type Gap,
  page,
  playwright,
} from './model'

const deep = { role: 'button', name: 'Deep', ref: 'e1' } as const

export const depth: Gap = {
  snapshots: {
    // 147 levels, the deepest tree Rust decodes, each level a line one step further in.
    'depth/147': chain({ depth: 147, link: { role: 'region' }, leaf: deep }),
    // 2,000 levels that print nothing in Neo's format: no walk recurses, and the leaf prints alone.
    'depth/2000': chain({ depth: 2000, link: { role: 'generic' }, leaf: deep }),
    // 200 levels in `mode: 'full'`, each a line one step further in: the cut counts every line of all 200.
    'depth/200': chain({ depth: 200, link: { role: 'generic' }, leaf: deep }),
  },
  cases: [
    ...calls('depth/147', {
      playwright: playwright(),
      full: full(),
      page: page(),
      'delta-uncompared': delta({}, 'notRead'),
    }),
    ...calls('depth/2000', {
      element: element(),
      page: page(),
      'delta-first': delta({}, 'nothing'),
    }),
    ...calls('depth/200', { full500: full(500) }),
  ],
}
