/**
 * The bundle entry `dist/utility.js`: `() => UtilityScript`, Playwright's main-world helper
 * (`server/javascript.ts` `ExecutionContext._utilityScript`: `new (module.exports.UtilityScript())
 * (globalThis, isUnderTest)`), built from the vendored, unchanged `utilityScript.ts`.
 *
 * neo-runtime's `main_world.rs` calls it with `Runtime.callFunctionOn` on the frame's main-world
 * `document` (resolved by `DOM.resolveNode` without an execution context), once per document and
 * session, and keeps the result only as a CDP object id, as Playwright keeps it as a handle. So
 * the page sees no new global: the object is reachable from Rust alone.
 *
 * The second file allowed to value-import vendored code (build.ts): the constructor is the point.
 */

import { UtilityScript } from '../vendor/playwright/injected/src/utilityScript'

export default function utility(): UtilityScript {
  // `isUnderTest: false`: Playwright's test-only `builtins` global is never written.
  return new UtilityScript(globalThis, false)
}
