// Each case must fail tsc. `@ts-expect-error` passes only when the next line has an error, and
// is itself an error when it does not, so this file type-checks only while every case still
// fails for the reason its comment gives.

import type {
  AriaNode,
  InjectedApi,
  InjectedInitializer,
} from '../../ts/injected'
import type { FrameGotoParams, FrameGotoResult } from '../../ts/serve'
import type { InjectedScript } from './injected-script'

/** The spec names no handle kinds besides the built-in `Element` yet. */
type Handles = Record<never, never>
type Api = InjectedApi<InjectedScript, Handles>

export const nullText = {
  title: () => ({ value: document.title }),
  textContent: () => ({
    // @ts-expect-error Node.textContent may be null, and the contract's text is a string.
    matched: { value: (document as Node).textContent, preview: '' },
  }),
  ariaSnapshot: () => ({}),
  frameOwner: () => ({}),
} satisfies Api

export const missingCall = {
  title: () => ({ value: document.title }),
  textContent: () => ({}),
  ariaSnapshot: () => ({}),
  // @ts-expect-error frameOwner is missing.
} satisfies Api

export const extraCall = {
  title: () => ({ value: document.title }),
  textContent: () => ({}),
  ariaSnapshot: () => ({}),
  frameOwner: () => ({}),
  // @ts-expect-error click is not a page call the contract declares.
  click: () => ({}),
} satisfies Api

export const valueForElement = {
  title: () => ({ value: document.title }),
  textContent: () => ({}),
  ariaSnapshot: () => ({}),
  // @ts-expect-error frameOwner returns the element itself, not a selector for it.
  frameOwner: () => ({ element: 'iframe#main' }),
} satisfies Api

export const unexpectedHandle = {
  title: () => ({ value: document.title }),
  textContent: () => ({}),
  ariaSnapshot: () => ({}),
  // @ts-expect-error frameOwner takes no handle argument: it finds its element by selector.
  frameOwner: (_injected, _params, element: Element) => ({ element }),
} satisfies Api

export const iframeRecord = {
  title: () => ({ value: document.title }),
  textContent: () => ({}),
  // @ts-expect-error iframes is a list, not Playwright's iframeDepths record.
  ariaSnapshot: () => ({ snapshot: { nodes: [], iframes: { e1: 0 } } }),
  frameOwner: () => ({}),
} satisfies Api

export const unknownParam = {
  title: () => ({ value: document.title }),
  textContent: (_injected, params) => ({
    // @ts-expect-error textContent's params have no selectors list.
    matched: { value: params.selectors[0], preview: '' },
  }),
  ariaSnapshot: () => ({}),
  frameOwner: () => ({}),
} satisfies Api

// @ts-expect-error Playwright has no "spaceship" role.
export const unknownRole: AriaNode = { role: 'spaceship' }

// @ts-expect-error checked is true, false or 'mixed'.
export const partlyChecked: AriaNode = { role: 'checkbox', checked: 'partly' }

export const textBox: AriaNode = {
  role: 'img',
  // @ts-expect-error a box has whole-pixel numbers, not strings.
  box: { x: '0', y: 0, width: 1, height: 1 },
}

// @ts-expect-error the initializer needs frameSeq.
export const noFrameSeq: InjectedInitializer = {
  testIdAttributeName: 'data-testid',
  stableRafCount: 1,
  isUtilityWorld: true,
}

export const gotoResponse: FrameGotoResult = {
  // @ts-expect-error Neo creates no Response objects, so goto never answers with one.
  response: { guid: 'response@1' },
}

export const gotoLater: FrameGotoParams = {
  url: 'https://a.test',
  // @ts-expect-error waitUntil is one of Playwright's lifecycle events.
  waitUntil: 'later',
}
