/**
 * Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`
 *
 * The bundle entry `dist/wait-result.js`: Playwright's `handle.evaluateHandle(h => h.result)` on
 * a `dist/wait-for-function.js` waiter, run through `UtilityScript.evaluate`, which awaits the
 * promise; the answer is a handle to the first truthy value.
 */

export default function waitResult(waiter: {
  result: Promise<unknown>
}): Promise<unknown> {
  return waiter.result
}
