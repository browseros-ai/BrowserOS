/**
 * Neo's own frame commands: browser calls Playwright's protocol does not have. Playwright's
 * client builds `waitForURL`, `waitForLoadState` and `waitForNavigation` itself, over the
 * server's `navigated` and `loadstate` events (`client/frame.ts` at 26137face). Neo's bridge
 * carries one reply per call and no events, so Rust owns each wait (`neo-runtime/src/waits.rs`
 * and `history.rs`: its deadline, its wake-ups, its cleanup), and the SDK sends one call that
 * replies once the wait is met.
 *
 * ```text
 *  page.waitForURL('https://a.test/done.html') → frame._waitForURL → Connection.sendNeo
 *    {"id":7,"guid":"frame@T","method":"neo:waitForURL",
 *     "params":{"url":{"glob":"https://a.test/done.html"},"waitUntil":"load"},
 *     "metadata":{"location":{…},"timeout":30000}}
 *    ── Rust: dispatch.rs decodes it before the spec's FrameCall; waits.rs waits ──▶
 *    ◀ {"id":7,"result":{}}  → undefined
 *
 *  page.waitForNavigation({ url: (url) => url.pathname === '/done' })
 *    neo:nextNavigation {response: true}   ◀ {cursor: 3, index: 0, url: '…/start#a'}  test it
 *    neo:nextNavigation {cursor: 3, …}     ◀ {cursor: 3, index: 1, url: '…/done'}     test it
 *    neo:nextNavigation {cursor: 3, …}     (waiting)       index 0: false; index 1: true
 *    neo:finishNavigation {cursor: 3, index: 1, waitUntil: 'load'}  ◀ {response: {guid}}
 *    (the waiting next answers {cursor: 3}: the wait ended)
 * ```
 *
 * The seam's decisions (the med-neo-waits design, DECIDED 1-3, and pw-net-nav):
 * 1. `locator.waitFor` is Playwright's own `Frame.waitForSelector`. Only the waits Playwright
 *    builds client-side are Neo commands; the `neo:` prefix keeps them clear of every Playwright
 *    command name, and the vendored protocol is untouched.
 * 2. These types are hand-written here and in Rust (`waits.rs` `NeoWaitForUrlParams`,
 *    `NeoWaitForLoadStateParams`, `history.rs` `NeoWaitForNavigationParams`,
 *    `NeoNextNavigationParams`, `NeoFinishNavigationParams`), not generated: both sides pin the
 *    exact JSON in a unit test (`tests/unit/waits.test.ts`, dispatch.rs's decode tests). Change
 *    one side, change both.
 * 3. `url` is a string glob or a `RegExp`, serialized as Playwright's `serializeURLMatch` without
 *    `urlPattern` (QuickJS has no `URLPattern`), and matched in Rust. A predicate function
 *    cannot cross: `waitForURL` and `waitForNavigation` step a cursor instead, one
 *    `neo:nextNavigation` per navigation and one always waiting, and test each URL in the realm
 *    on its own, as Playwright's waiter does: a pending answer holds back no later navigation, and
 *    the waiting call carries the page's closure and failed navigations to the wait. Every exit
 *    ends the cursor (`neo:finishNavigation`, `neo:releaseNavigation`, or a failed step).
 *    `page.frame({ url })` still refuses one.
 */

import type {
  LifecycleEvent,
  PageScreenshotParams,
  ResponseChannel,
} from '@browseros/neo-runtime-spec/serve'
import { isString } from '../vendor/playwright/isomorphic/stringUtils'
import type { ApiName } from './connection'

/** Playwright's `SerializedURLMatch` (`isomorphic/urlMatch.ts`) without `urlPattern`. A glob of
 * `''` matches every URL. */
export type NeoSerializedUrlMatch =
  | { glob: string }
  | { regexSource: string; regexFlags: string }

/** `neo:waitForURL`: once the main frame's URL has matched `url`, resolves when `waitUntil` fires,
 * at once if the current document already fired it, as Playwright's `waitForURL` does. Rust
 * refuses `networkidle`. */
export type NeoWaitForURLParams = {
  url: NeoSerializedUrlMatch
  waitUntil: LifecycleEvent
}

/** `neo:waitForLoadState`: resolves once `state` fired for the main frame's current document,
 * or for the next one. Rust refuses `networkidle`. */
export type NeoWaitForLoadStateParams = { state: LifecycleEvent }

/** `neo:screenshot`: `locator.screenshot`. Playwright's client resolves the locator to an
 * element (`waitForSelector`, strict, `attached`) and calls `ElementHandle.screenshot` on it; Neo
 * has no element handles yet, so one call carries the selector, `ElementHandle.screenshot`'s
 * params, and Neo's `path` (relative to the run's artifact directory). Rust: `capture.rs`
 * `NeoScreenshotParams`; the reply is `ElementHandle.screenshot`'s `{binary}`. */
export type NeoScreenshotParams = ElementScreenshotParams & { selector: string }

/** `elementHandle.screenshot`'s params: `ElementHandle.screenshot`'s
 * (`spec/playwright/handles.yml`, `PageScreenshotParams` without `fullPage` and `clip`) and
 * Neo's `path`. The reply is `{binary}`. */
export type ElementScreenshotParams = Omit<
  PageScreenshotParams,
  'fullPage' | 'clip'
> & {
  path?: string
}

/** `neo:scrollIntoViewIfNeeded` and `neo:boundingBox`: Playwright's `Locator` methods that its
 * client reaches through an `ElementHandle` (`_withElement`), which Rust serves whole
 * (`neo-runtime/src/element_ops.rs`). */
export type NeoElementParams = { selector: string; strict: boolean }

/** `neo:selectText`, with `ElementHandle.selectText`'s `force`. */
export type NeoSelectTextParams = NeoElementParams & { force?: boolean }

/** `neo:selectText`'s params in `ElementHandle.selectText`'s schema order; `pickParams` turns
 * a `null` `force` into Playwright's validator error. */
export const NEO_SELECT_TEXT_KEYS = ['selector', 'strict', 'force'] as const

/** Playwright's `Rect`, in the main frame's CSS pixels. */
export type NeoRect = { x: number; y: number; width: number; height: number }

/** `neo:waitForNavigation`: Playwright's `waitForNavigation` for the main frame with a glob or
 * `RegExp` `url` (`''` for none): the first navigation after the call whose URL matches (a
 * same-document one counts; a failed one rejects), then `waitUntil`. Rust refuses `networkidle`
 * and child frames, whose navigation requests it does not record. */
export type NeoWaitForNavigationParams = {
  url: NeoSerializedUrlMatch
  waitUntil: LifecycleEvent
}

/** What a navigation wait answers: the new document's `Response`, absent for `null`. */
export type NeoNavigationResult = { response?: ResponseChannel }

/** `neo:nextNavigation`: the next navigation of a URL-predicate wait, whose URL the realm tests.
 * Without `cursor` it opens the wait on this frame, subscribed before the call returns anything;
 * `response` says whether its finish answers the navigation's `Response` (`waitForNavigation`,
 * main frame only) or nothing (`waitForURL`). A failed navigation, the page's closure or the
 * call's timeout rejects, and ends the wait. */
export type NeoNextNavigationParams = { cursor?: number; response: boolean }

/** The navigation `neo:nextNavigation` saw, by its index in the cursor, and the cursor to continue
 * or finish with; no `index` once the wait ended under the call (its finish or release). */
export type NeoNextNavigationResult = {
  cursor: number
  index?: number
  url?: string
}

/** `neo:finishNavigation`: ends the cursor with navigation `index`, the first the predicate
 * accepted: `waitUntil` since it, then its `Response` when the cursor answers one. */
export type NeoFinishNavigationParams = {
  cursor: number
  index: number
  waitUntil: LifecycleEvent
}

/** `neo:releaseNavigation`: ends a cursor whose wait ended in the realm, stopping its waiting
 * `neo:nextNavigation`. */
export type NeoReleaseNavigationParams = { cursor: number }

/** Neo's commands on a frame guid, shaped as the generated `FrameCommands`;
 * `neo:boundingBox`'s `value` is absent when the element has no box. */
export type NeoFrameCommands = {
  'neo:scrollIntoViewIfNeeded': {
    params: NeoElementParams
    result: Record<string, never>
  }
  'neo:selectText': {
    params: NeoSelectTextParams
    result: Record<string, never>
  }
  'neo:boundingBox': {
    params: NeoElementParams
    result: { value?: NeoRect }
  }
  'neo:waitForURL': {
    params: NeoWaitForURLParams
    result: Record<string, never>
  }
  'neo:waitForLoadState': {
    params: NeoWaitForLoadStateParams
    result: Record<string, never>
  }
  'neo:screenshot': {
    params: NeoScreenshotParams
    result: { binary: string }
  }
  'neo:waitForNavigation': {
    params: NeoWaitForNavigationParams
    result: NeoNavigationResult
  }
  'neo:nextNavigation': {
    params: NeoNextNavigationParams
    result: NeoNextNavigationResult
  }
  'neo:finishNavigation': {
    params: NeoFinishNavigationParams
    result: NeoNavigationResult
  }
  'neo:releaseNavigation': {
    params: NeoReleaseNavigationParams
    result: Record<string, never>
  }
}

/**
 * Playwright's `serializeURLMatch` for `waitForURL`'s `url`, run before anything is sent: a
 * string (or `String` object, as Playwright's `isString`) is a glob and a `RegExp` its `source`
 * and `flags`. `undefined` is the `''` glob: Playwright's `urlMatches` matches every URL for both.
 * An empty `String` object is not `''` there (`match === ''`): it becomes `new RegExp('^$')`,
 * which no URL matches, and is sent as exactly that.
 *
 * A function is Neo's limit (DECIDED 3) and throws an `Error` prefixed with `apiName`. Any other
 * value throws Playwright's own `urlMatches` error, which no API wrapper prefixes.
 */
export function serializeUrlMatch(
  apiName: ApiName,
  url: unknown,
): NeoSerializedUrlMatch {
  if (url === undefined || url === '') return { glob: '' }
  if (isString(url))
    return url.length === 0
      ? { regexSource: '^$', regexFlags: '' }
      : { glob: url }
  if (isRegExp(url)) return { regexSource: url.source, regexFlags: url.flags }
  if (typeof url === 'function')
    throw new Error(
      `${apiName}: Neo does not support a predicate function as url yet; pass a glob string or a RegExp`,
    )
  throw new Error(
    'url parameter should be string, RegExp, URLPattern or function',
  )
}

/** Playwright's `isRegExp` (`isomorphic/urlMatch.ts`), which that module does not export. */
function isRegExp(value: unknown): value is RegExp {
  return (
    value instanceof RegExp ||
    Object.prototype.toString.call(value) === '[object RegExp]'
  )
}
