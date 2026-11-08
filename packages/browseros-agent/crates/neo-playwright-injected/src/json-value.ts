/**
 * Playwright: `server/javascript.ts` `JSHandle._jsonValue`
 *
 * The bundle entry `dist/json-value.js`: `(utilityScript, ...args) =>
 * utilityScript.jsonValue(...args)`, called by neo-runtime's `main_world.rs` with
 * `[true, handle]` for `JSHandle.jsonValue`.
 */

import type { UtilityScript } from '../vendor/playwright/injected/src/utilityScript'

export default function jsonValue(
  utilityScript: UtilityScript,
  ...args: Parameters<UtilityScript['jsonValue']>
): unknown {
  return utilityScript.jsonValue(...args)
}
