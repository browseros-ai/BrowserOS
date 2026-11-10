/**
 * Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`
 *
 * The bundle entry `dist/wait-abort.js`: Playwright's `handle.evaluate(h => h.abort())`, which
 * stops a `dist/wait-for-function.js` waiter's polling once its call timed out or ended.
 */

export default function waitAbort(waiter: { abort: () => void }): void {
  waiter.abort()
}
