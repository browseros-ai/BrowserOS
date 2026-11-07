/**
 * Playwright: `server/javascript.ts` `evaluateExpression`
 *
 * The bundle entry `dist/evaluate.js`: the calling convention of every page evaluation,
 * `(utilityScript, ...args) => utilityScript.evaluate(...args)`. neo-runtime's `main_world.rs`
 * passes it as `Runtime.callFunctionOn`'s `functionDeclaration` on the frame's `UtilityScript`,
 * with the arguments Playwright's Chromium execution context sends: the utility script,
 * `isFunction`, `returnByValue`, the expression, the argument count, the serialized arguments,
 * then the handles.
 *
 * The agent's code travels only as the `expression` argument's value; this function is the only
 * thing CDP compiles from Neo for it.
 */

import type { UtilityScript } from '../vendor/playwright/injected/src/utilityScript'

export default function evaluate(
  utilityScript: UtilityScript,
  ...args: Parameters<UtilityScript['evaluate']>
): unknown {
  return utilityScript.evaluate(...args)
}
