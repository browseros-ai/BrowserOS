/**
 * Playwright: `server/frames.ts` `Frame._waitForFunctionOnSelector`
 * Playwright: `server/frameSelectors.ts` `FrameSelectors._callOnSelectorInternal`
 *
 * The bundle entry `dist/call-on-element.js`, run as `UtilityScript.evaluate`'s expression in the
 * page's own world with that world's `InjectedScript` (`dist/main-injected.js`): the page side of
 * Playwright's `mainWorld: true` element callbacks. It queries the selector (within `scope` when
 * given) with Playwright's checks in Playwright's order (mark the first match, reject a
 * deprecated selector, strict mode), then runs the named callback on the first match. It answers
 * `0` when nothing matches and `1` when the callback's value is falsy (the caller polls again),
 * else the match's preview for the call log. The callbacks are Playwright's own, by name: agent
 * code reaches the page only as their arguments.
 */

import type { InjectedScript } from '../vendor/playwright/injected/src/injectedScript'

type Callback = (
  injected: InjectedScript,
  element: Element,
  ...data: unknown[]
) => unknown

const CALLBACKS: Record<'dispatchEvent' | 'waitForFunction', Callback> = {
  // Playwright: `server/frames.ts` `Frame.dispatchEvent`: `injectedScript.dispatchEvent(element, data.type, data.eventInit);`
  dispatchEvent: (injected, element, type, eventInit) => {
    injected.dispatchEvent(element, type as string, eventInit as object)
    return { result: undefined }
  },
  // Playwright: `server/frames.ts` `Frame.waitForFunctionExpressionOnElement`: `let result = globalThis.eval(data.expression);`
  waitForFunction: (_injected, element, expression, isFunction, arg) => {
    // biome-ignore lint/security/noGlobalEval: Playwright's page callback evaluates the agent's expression
    let result: unknown = globalThis.eval(expression as string)
    const call =
      isFunction === true ||
      (isFunction === undefined && typeof result === 'function')
    if (call)
      result = (result as (element: Element, arg: unknown) => unknown)(
        element,
        arg,
      )
    return result
  },
}

/** Playwright: `server/frameSelectors.ts` `FrameSelectors._callOnSelectorInternal`: `const elements = injected.querySelectorAll(params.info.parsed, params.scope || document);` */
export default function callOnElement(
  injected: InjectedScript,
  selector: string,
  strict: boolean,
  scope: Element | undefined,
  callback: keyof typeof CALLBACKS,
  ...data: unknown[]
): 0 | 1 | string {
  const parsed = injected.parseSelector(selector)
  const elements = injected.querySelectorAll(parsed, scope || document)
  if (elements.length) injected.markTargetElements(new Set([elements[0]]))
  else injected.markTargetElements(new Set())
  injected.checkDeprecatedSelectorUsage(parsed, elements)
  if (strict && elements.length > 1)
    throw injected.strictModeViolationError(parsed, elements)
  const [element] = elements
  if (!element) return 0
  if (!CALLBACKS[callback](injected, element, ...data)) return 1
  return injected.previewNode(element)
}
