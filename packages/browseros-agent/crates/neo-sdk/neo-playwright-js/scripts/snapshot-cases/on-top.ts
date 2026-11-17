/**
 * The on-top lines: a layer Rust words is printed first, a whole-view layer's content moves to the front, and a delta says `# no longer on top:`.
 */
import type { AriaNodeJSON } from '../../src/prelude/vendored'
import {
  calls,
  delta,
  type Facts,
  full,
  type Gap,
  keptBy,
  page,
  playwright,
  tree,
} from './model'

type Layer = NonNullable<Facts['onTop']>[number]

const tickets = (reserve = 'Reserve'): AriaNodeJSON => ({
  role: 'main',
  ref: 'e2',
  children: [{ role: 'button', name: reserve, ref: 'e3' }],
})

const welcome = (...rest: AriaNodeJSON[]): AriaNodeJSON[] => [
  {
    role: 'generic',
    ref: 'e1',
    children: ['Welcome back.', tickets(), ...rest],
  },
]

const promo = (): AriaNodeJSON => ({
  role: 'generic',
  ref: 'e9',
  onTop: 'e9',
  children: [
    'Sale ends today.',
    { role: 'button', name: 'Shop', ref: 'e10' },
    'Use code SAVE.',
  ],
})

const PROMO: Layer = {
  ref: 'e9',
  line: '# on top: generic [ref=e9] over the whole view\n#   its controls: button "Shop" [ref=e10]',
  wholeView: true,
}

const signIn = (marked: boolean): AriaNodeJSON => ({
  role: 'dialog',
  name: 'Sign in',
  ref: 'e12',
  ...(marked ? { onTop: 'e12' } : {}),
  children: [
    'Use your account.',
    { role: 'button', name: 'Continue', ref: 'e13' },
  ],
})

const SIGN_IN: Layer = {
  ref: 'e12',
  line: '# on top: dialog "Sign in" [ref=e12] over the whole view\n#   its controls: button "Continue" [ref=e13]',
  wholeView: true,
}

const FORMS: Record<string, Layer> = {
  controls: {
    ref: 'e11',
    line: '# on top: dialog "Cookies" [ref=e11] over part of the view\n#   its controls: button "Accept" [ref=e12], button "Reject all cookies except the ones this …" [ref=e13], button "Close" [ref=e15], link "Settings" [ref=e14]',
    wholeView: false,
  },
  quoted: {
    ref: 'e21',
    line: '# on top: dialog "Say \\"yes\\" [ref=e5] to C:\\\\" [ref=e21] over the whole view',
    wholeView: true,
  },
  clipped: {
    ref: 'e31',
    line: '# on top: region "Our spring sale is on: every seat in every hall is ten percent off till Friday 🎟…" [ref=e31] over part of the view',
    wholeView: false,
  },
  nameless: {
    ref: 'e41',
    line: '# on top: generic [ref=e41] over the whole view',
    wholeView: true,
  },
  blank: {
    ref: 'e51',
    line: '# on top: generic [ref=e51], blank over the whole view',
    wholeView: true,
  },
  loading: {
    ref: 'e61',
    line: '# on top: dialog "Checkout" [ref=e61], loading over the whole view\n#   its controls: button "Cancel" [ref=e62], button [ref=e63]',
    wholeView: true,
  },
  says: {
    ref: 'e71',
    line: '# on top: generic [ref=e71], says "Sale ends today." over part of the view',
    wholeView: false,
  },
  'blank-loading': {
    ref: 'e81',
    line: '# on top: generic [ref=e81], blank, loading over the whole view',
    wholeView: true,
  },
  'loading-says-clipped': {
    ref: 'e91',
    line: '# on top: generic [ref=e91], loading, says "Wait: we are \\"holding\\" your seats while the payment goes thr…" over part of the view',
    wholeView: false,
  },
}

export const onTop: Gap = {
  snapshots: {
    // A whole-view layer: its lines first, its content first, its emptied region gone, the page's own text kept apart.
    'on-top/whole': tree(
      welcome({
        role: 'region',
        name: 'Promo',
        ref: 'e8',
        children: [promo()],
      }),
      { onTop: [PROMO] },
    ),
    // A part-view layer: its lines first, its content where the page has it.
    'on-top/part': tree(
      welcome({
        role: 'dialog',
        name: 'Cookies',
        ref: 'e9',
        children: [{ role: 'button', name: 'Accept', ref: 'e10' }],
      }),
      {
        onTop: [
          {
            ref: 'e9',
            line: '# on top: dialog "Cookies" [ref=e9] over part of the view\n#   its controls: button "Accept" [ref=e10]',
            wholeView: false,
          },
        ],
      },
    ),
    // Two whole-view layers with one ref: both lines, and the block printed twice.
    'on-top/one-ref-twice': tree(welcome(signIn(true)), {
      onTop: [SIGN_IN, SIGN_IN],
    }),
    // The same page with the layers gone: a `# no longer on top:` line for each line that named the ref.
    'on-top/one-ref-gone': tree(welcome(signIn(false))),
    // Two whole-view layers: their blocks in the order of the facts, not of the page.
    'on-top/two-layers': tree(welcome(promo(), signIn(true)), {
      onTop: [SIGN_IN, PROMO],
    }),
    // A whole-view layer marked inert is not moved: left out and counted, and in place under { inert: true }.
    'on-top/inert-layer': tree(
      welcome({ ...promo(), inert: true }, signIn(true)),
      { onTop: [SIGN_IN, PROMO] },
    ),
    // A whole-view layer under two wrappers that each hold only the next: every wrapper is
    // replaced, so the layer's content prints once, first.
    'on-top/whole-deep': tree(
      [
        {
          role: 'generic',
          children: [{ role: 'generic', children: [signIn(true)] }],
        },
        tickets(),
      ],
      { onTop: [SIGN_IN] },
    ),
    // Every form of the words, with no content marked.
    'on-top/every-form': tree([tickets()], { onTop: Object.values(FORMS) }),
    // The same page under no layer: each form comes back as `# no longer on top:` with its own ref.
    'on-top/no-layers': tree([tickets()]),
    // Some layers stay, one with new words for its ref, and a button is renamed: the others are no longer on top, then the hunk.
    'on-top/some-layers': tree([tickets('Book')], {
      onTop: [
        FORMS.controls,
        {
          ref: 'e41',
          line: '# on top: generic [ref=e41], loading over the whole view',
          wholeView: true,
        },
        FORMS.says,
      ],
    }),
  },
  cases: [
    ...calls('on-top/whole', {
      playwright: playwright(),
      full: full(),
      page: page(),
      'page-notext': page({ text: false }),
      'delta-first': delta({}, 'nothing'),
      'delta-uncompared': delta({}, 'notRead'),
    }),
    ...calls('on-top/part', { page: page() }),
    ...calls('on-top/one-ref-twice', { page: page() }),
    ...calls('on-top/one-ref-gone', {
      'delta-after-one-ref-twice': delta({}, keptBy('on-top/one-ref-twice')),
    }),
    ...calls('on-top/two-layers', { page: page() }),
    ...calls('on-top/inert-layer', {
      page: page(),
      'page-inert': page({ inert: true }),
    }),
    ...calls('on-top/whole-deep', { page: page() }),
    ...calls('on-top/no-layers', {
      'delta-after-every-form': delta({}, keptBy('on-top/every-form')),
    }),
    ...calls('on-top/some-layers', {
      'delta-after-every-form': delta({}, keptBy('on-top/every-form')),
    }),
  ],
}
