/**
 * Neo: the standard globals a Node Playwright script uses and QuickJS lacks. `createRun` hands
 * them over and `quickjs.rs` defines them on the realm's global object before the agent's script
 * runs, so evaluating the bundle still defines no globals. `URL`, `Buffer` and the rest are the
 * very classes the SDK itself uses, so a screenshot is `instanceof Buffer` and a URL predicate's
 * argument `instanceof URL`.
 *
 * Not served, so `typeof` reads `'undefined'` as before: `Intl` (QuickJS-ng has none, and ICU's
 * data is tens of megabytes) and `fetch` (it needs a decision on which network a script may
 * reach, tied to `APIRequestContext`).
 */

import { Buffer } from './buffer'
import { structuredClone } from './structured-clone'
import { TextDecoder, TextEncoder } from './text'
import { URL, URLSearchParams } from './url'
import { crypto } from './web-crypto'

/** The globals `quickjs.rs` defines, by name. */
export function standardGlobals(): Record<string, unknown> {
  return {
    URL,
    URLSearchParams,
    Buffer,
    TextEncoder,
    TextDecoder,
    structuredClone,
    crypto,
  }
}
