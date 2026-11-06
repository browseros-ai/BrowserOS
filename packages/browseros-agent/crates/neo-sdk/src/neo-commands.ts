/**
 * Neo's own frame commands: browser calls Playwright's protocol does not have. Playwright's
 * client builds `waitForURL` and `waitForLoadState` itself, over the server's `navigated` and
 * `loadstate` events (`client/frame.ts` at 26137face). Neo's bridge carries one reply per call and
 * no events, so Rust owns each whole wait (`neo-runtime/src/waits.rs`: its deadline, its wake-ups,
 * its cleanup), and the SDK sends one call that replies `{}` once the wait is met.
 *
 * ```text
 *  page.waitForURL('https://a.test/done.html') → frame._waitForURL → Connection.sendNeo
 *    {"guid":"frame@T","method":"neo:waitForURL","metadata":{…,"timeout":30000},
 *     "params":{"url":{"glob":"https://a.test/done.html"},"waitUntil":"load"}}
 *    ── Rust: dispatch.rs decodes it before the spec's FrameCall; waits.rs waits ──▶
 *    ◀ {"id":7,"result":{}}  → undefined
 * ```
 *
 * The seam's decisions (the med-neo-waits design, DECIDED 1-3):
 * 1. `locator.waitFor` is Playwright's own `Frame.waitForSelector`. Only the two waits Playwright
 *    builds client-side are Neo commands; the `neo:` prefix keeps them clear of every Playwright
 *    command name, and the vendored protocol is untouched.
 * 2. These types are hand-written here and in Rust (`waits.rs` `NeoWaitForUrlParams`,
 *    `NeoWaitForLoadStateParams`), not generated: both sides pin the exact JSON in a unit test
 *    (`tests/unit/waits.test.ts`, waits.rs's decode tests). Change one side, change both.
 * 3. `url` is a string glob or a `RegExp`, serialized as Playwright's `serializeURLMatch` without
 *    `urlPattern` (QuickJS has no `URLPattern`). A predicate function cannot cross: it would have
 *    to run in the realm on every navigation while the call is parked in Rust, so the SDK refuses
 *    one before sending, never matching something else in its place.
 */

import type { LifecycleEvent } from '@browseros/neo-runtime-spec/serve'
import { isString } from '../vendor/playwright/isomorphic/stringUtils'
import type { ApiName } from './connection'

/** Playwright's `SerializedURLMatch` (`isomorphic/urlMatch.ts`) without `urlPattern`. A glob of
 * `''` matches every URL. */
export type NeoSerializedUrlMatch =
  | { glob: string }
  | { regexSource: string; regexFlags: string }

/** `neo:waitForURL`: resolves once the main frame's URL matches `url` and `waitUntil` fired for
 * that document. Rust refuses `networkidle`. */
export type NeoWaitForURLParams = {
  url: NeoSerializedUrlMatch
  waitUntil: LifecycleEvent
}

/** `neo:waitForLoadState`: resolves once `state` fired for the main frame's current document,
 * or for the next one. Rust refuses `networkidle`. */
export type NeoWaitForLoadStateParams = { state: LifecycleEvent }

/** Neo's commands on a frame guid, shaped as the generated `FrameCommands`. Each replies `{}`. */
export type NeoFrameCommands = {
  'neo:waitForURL': {
    params: NeoWaitForURLParams
    result: Record<string, never>
  }
  'neo:waitForLoadState': {
    params: NeoWaitForLoadStateParams
    result: Record<string, never>
  }
}

/**
 * Playwright's `serializeURLMatch` for `waitForURL`'s `url`, run before anything is sent: a
 * string (or `String` object, as Playwright's `isString`) is a glob and a `RegExp` its `source`
 * and `flags`. `undefined` is the `''` glob: Playwright's `urlMatches` matches every URL for both.
 *
 * A function is Neo's limit (DECIDED 3) and throws an `Error` prefixed with `apiName`. Any other
 * value throws Playwright's own `urlMatches` error, which no API wrapper prefixes.
 */
export function serializeUrlMatch(
  apiName: ApiName,
  url: unknown,
): NeoSerializedUrlMatch {
  if (url === undefined) return { glob: '' }
  if (isString(url)) return { glob: url }
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
