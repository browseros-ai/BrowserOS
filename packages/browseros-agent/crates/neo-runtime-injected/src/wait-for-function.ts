/**
 * Playwright: `server/frames.ts` `Frame.waitForFunctionExpression`
 *
 * The bundle entry `dist/wait-for-function.js`: the page half of `page.waitForFunction`, Playwright's
 * own callback with the injected script's builtins replaced by the `UtilityScript`'s (the same
 * page functions, saved before the page could replace them). It polls the agent's predicate on
 * every animation frame, or every `polling` ms, and returns `{ result, abort }`: `result`
 * resolves with the first truthy value. neo-runtime's `main_world.rs` passes it as the
 * `expression` of `UtilityScript.evaluate`, so the agent's argument arrives parsed, handles
 * included; `dist/wait-result.js` awaits `result`, `dist/wait-abort.js` stops the polling.
 */

import type { UtilityScript } from '../vendor/playwright/injected/src/utilityScript'

type Waiter = { result: Promise<unknown>; abort: () => void }

export default function waitForFunction(
  utilityScript: UtilityScript,
  expression: string,
  isFunction: boolean | undefined,
  polling: number | undefined,
  arg: unknown,
): Waiter {
  let evaledExpression: unknown
  const predicate = (): unknown => {
    // biome-ignore lint/security/noGlobalEval: Playwright's page callback evaluates the agent's expression
    let result: unknown = evaledExpression ?? globalThis.eval(expression)
    if (isFunction === true) {
      evaledExpression = result
      result = (result as (arg: unknown) => unknown)(arg)
    } else if (isFunction === false) {
      // The expression's value, read again on every poll.
    } else if (typeof result === 'function') {
      // Auto detect.
      evaledExpression = result
      result = (result as (arg: unknown) => unknown)(arg)
    }
    return result
  }

  let fulfill: (result: unknown) => void = () => {}
  let reject: (error: unknown) => void = () => {}
  let aborted = false
  const result = new Promise<unknown>((f, r) => {
    fulfill = f
    reject = r
  })

  const next = () => {
    if (aborted) return
    try {
      const success = predicate()
      if (success) {
        fulfill(success)
        return
      }
      if (typeof polling !== 'number')
        utilityScript.builtins.requestAnimationFrame(next)
      else utilityScript.builtins.setTimeout(next, polling)
    } catch (e) {
      reject(e)
    }
  }

  next()
  return {
    result,
    abort: () => {
      aborted = true
    },
  }
}
