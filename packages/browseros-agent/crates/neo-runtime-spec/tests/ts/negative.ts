// Each case must fail tsc. `@ts-expect-error` passes only when the next line has an error, and
// is itself an error when it does not, so this file type-checks only while every case still
// fails for the reason its comment gives.

import type {
  AriaNode,
  InjectedApi,
  InjectedInitializer,
} from '../../ts/injected'
import type {
  FrameGotoParams,
  FrameGotoResult,
  RequestInitializer,
} from '../../ts/serve'
import type { InjectedScript } from './injected-script'

/** The named handle kinds, bound as the real glue binds them to Playwright's page types. */
type Handles = {
  HitTargetInterceptor: {
    stop: () => 'done' | { hitTargetDescription: string }
  }
}
type Api = InjectedApi<InjectedScript, Handles>

/** A table that satisfies the contract; each case below breaks one entry of it. */
const valid = {
  title: () => ({ value: document.title }),
  selectorState: () => ({}),
  textContent: () => ({}),
  innerText: () => ({}),
  getAttribute: () => ({}),
  isVisible: () => ({ value: false }),
  isChecked: () => ({}),
  ariaSnapshot: () => ({}),
  frameOwner: () => ({ visible: false }),
  frameOwnerRef: () => ({}),
  describeSelector: (_injected, { selector }) => ({ locator: selector }),
  resolve: () => ({ count: 0 }),
  checkElementStates: () => ({}),
  scrollIntoView: () => ({}),
  viewportSize: () => ({ width: 1280, height: 720 }),
  getElementBorderWidth: () => ({ left: 0, top: 0 }),
  describeIFrameStyle: () => ({ style: { left: 0, top: 0 } }),
  expectHitTarget: () => ({ outcome: 'done' }),
  setupHitTargetInterceptor: () => ({ outcome: 'error:notconnected' }),
  stopHitTargetInterceptor: (_injected, _params, interceptor) => ({
    outcome: interceptor.stop(),
  }),
  fill: () => ({ outcome: 'done' }),
  selectOptions: () => ({ values: [] }),
  elementState: () => ({ matches: false }),
  focusElement: () => ({ outcome: 'done' }),
  inputValue: () => ({}),
  queryCount: () => ({ value: 0 }),
  allTextContents: () => ({ values: [] }),
} satisfies Api

export const unknownFillOutcome = {
  ...valid,
  // @ts-expect-error fill returns needsinput, done, notconnected or a missing state.
  fill: () => ({ outcome: 'filled' }),
} satisfies Api

export const wrongSelectValues = {
  ...valid,
  // @ts-expect-error selected values cross by value as strings, never element handles.
  selectOptions: () => ({ values: [document.body] }),
} satisfies Api

export const nullText = {
  ...valid,
  textContent: () => ({
    // @ts-expect-error Node.textContent may be null, and the contract's text is a string.
    matched: { value: (document as Node).textContent, preview: '' },
  }),
} satisfies Api

const { frameOwner: _frameOwner, ...withoutFrameOwner } = valid
// @ts-expect-error frameOwner is missing.
export const missingCall = withoutFrameOwner satisfies Api

export const extraCall = {
  ...valid,
  // @ts-expect-error click is not a page call the contract declares.
  click: () => ({}),
} satisfies Api

export const valueForElement = {
  ...valid,
  // @ts-expect-error frameOwner returns the element itself, not a selector for it.
  frameOwner: () => ({ element: 'iframe#main' }),
} satisfies Api

export const unexpectedHandle = {
  ...valid,
  // @ts-expect-error frameOwner takes no handle argument: it finds its element by selector.
  frameOwner: (_injected, _params, element: Element) => ({ element }),
} satisfies Api

export const elementAsInterceptor = {
  ...valid,
  // @ts-expect-error stopHitTargetInterceptor stops the interceptor that setup armed, not an element.
  stopHitTargetInterceptor: (_injected, _params, interceptor: Element) => ({
    outcome: interceptor.isConnected ? 'done' : 'done',
  }),
} satisfies Api

export const elementAsArmed = {
  ...valid,
  setupHitTargetInterceptor: (_injected, _params, element) => ({
    // @ts-expect-error an armed outcome is the interceptor, not the element it guards.
    outcome: element,
  }),
} satisfies Api

export const unknownState = {
  ...valid,
  // @ts-expect-error a missing state is one of Playwright's element states.
  checkElementStates: () => ({ outcome: { missingState: 'focused' } }),
} satisfies Api

export const iframeRecord = {
  ...valid,
  // @ts-expect-error iframes is a list, not Playwright's iframeDepths record.
  ariaSnapshot: () => ({ snapshot: { nodes: [], iframes: { e1: 0 } } }),
} satisfies Api

export const unknownParam = {
  ...valid,
  textContent: (_injected, params) => ({
    // @ts-expect-error textContent's params have no selectors list.
    matched: { value: params.selectors[0], preview: '' },
  }),
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
  // @ts-expect-error a Response is named by its guid only.
  response: { guid: 'response@T:L:0', status: 200 },
}

export const workerRequest: RequestInitializer = {
  url: 'https://a.test',
  resourceType: 'document',
  method: 'GET',
  headers: [],
  isNavigationRequest: true,
  // @ts-expect-error Neo serves no Worker objects, so a request names no service worker.
  serviceWorker: { guid: 'worker@1' },
}

export const gotoLater: FrameGotoParams = {
  url: 'https://a.test',
  // @ts-expect-error waitUntil is one of Playwright's lifecycle events.
  waitUntil: 'later',
}
