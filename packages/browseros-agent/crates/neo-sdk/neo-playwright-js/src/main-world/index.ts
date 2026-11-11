/**
 * The function crates/neo-sdk/neo-playwright/src/page/evaluate_guard.rs sends as
 * `Runtime.callFunctionOn`'s declaration for every agent script in a page's main world. The build
 * (scripts/build.ts) bundles it with the vendored UtilityScript and its serializers into
 * crates/neo-sdk/neo-playwright/assets/main-world.js, one function expression.
 *
 * It runs upstream's own page code where upstream's page runs it: `UtilityScript.evaluate`
 * verbatim, and `waitForFunction`'s predicate as the server builds it (`waitForFunctionExpression`,
 * PW:25672-25722). What it adds is the transport. Values cross CDP as JSON text in Playwright's
 * serialized format, both ways, so a message stays flat however deep its value: browseros-cdp
 * drops a reply nested deeper than 128 levels. Rust parses that text strictly and masks it by its
 * structure before anything leaves the op (`page/page_value.rs`). json-text.ts reads and writes
 * the text, not the page's `JSON`, and the arguments are walked by index, not with the array
 * methods: the page owns those globals and may have replaced them, and upstream, whose values
 * cross CDP by value, never touches them.
 *
 * Nothing of Neo stays in the page. The bundle travels with every call, and the poller lives only
 * in the op's CDP object group, which Rust releases when the op ends.
 *
 *   evaluate  (isFunction, expression, argCount, ...argTexts, ...handles)
 *             → UtilityScript.evaluate → the result's serialized text, or undefined
 *   truthy    (isFunction, expression, argCount, ...argTexts, ...handles)
 *             → 'true' or 'false': whether the result is truthy, judged as it was returned, as
 *               upstream's `waitForFunctionExpressionOnElement` callback judges it (PW frames.ts):
 *               a promise is truthy and is not awaited
 *   poller    (isFunction, expression, argCount, ...argTexts, ...handles)
 *             → the poll function, by reference, its first poll already run (`first`, so the
 *               agent's code runs in this call, with its user gesture); each call of it judges
 *               the predicate in the page: [false] to keep waiting, [true, text] once it
 *               settled truthy, its value kept as `last`
 */
import { UtilityScript } from '../../../neo-playwright/third_party/playwright/packages/injected/src/utilityScript'
import {
  parseEvaluationResultValue,
  type SerializedValue,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers'
import { readJson, writeJson } from './json-text'

/** What a poll answers: keep waiting, or the settled value's serialized text. */
type PollOutcome = [false] | [true, string | undefined]

/**
 * The poller Rust holds by reference: `first` is what its first poll answered, and `last` the
 * value it settled with, for Rust to read by reference.
 */
type Poller = (() => PollOutcome | Promise<PollOutcome>) & {
  first?: PollOutcome | Promise<PollOutcome>
  last?: unknown
}

/** upstream's own thenable test (`UtilityScript._promiseAwareJsonValueNoThrow`). */
const isThenable = (value: unknown): value is PromiseLike<unknown> =>
  !!value &&
  typeof value === 'object' &&
  typeof (value as { then?: unknown }).then === 'function'

/**
 * JSON text, so the CDP reply carries one string. `undefined` stays `undefined`: the serializer
 * gives it for a value it drops (a function), and CDP reports it as no value.
 */
const asText = (value: SerializedValue | undefined): string | undefined =>
  value === undefined ? undefined : writeJson(value)

export default function neoMainWorld(
  mode: 'evaluate' | 'truthy' | 'poller',
  isFunction: boolean | undefined,
  expression: string,
  argCount: number,
  ...argTextsThenHandles: unknown[]
): unknown {
  const args: SerializedValue[] = []
  for (let index = 0; index < argCount; index++)
    args[index] = readJson(
      argTextsThenHandles[index] as string,
    ) as SerializedValue
  const handles = argTextsThenHandles.slice(argCount)
  const utility = new UtilityScript(globalThis, false)
  if (mode === 'evaluate') {
    const out: unknown = utility.evaluate(
      isFunction,
      true,
      expression,
      argCount,
      ...args,
      ...handles,
    )
    return isThenable(out)
      ? out.then((value) => asText(value as SerializedValue | undefined))
      : asText(out as SerializedValue | undefined)
  }
  if (mode === 'truthy') {
    // `locator.waitForFunction`'s try on the element its locator matched, as upstream's server
    // evaluates its expression and calls it with the element and the argument. Rust reads the
    // answer as a serialized boolean.
    const element =
      argCount > 0 ? parseEvaluationResultValue(args[0], handles) : undefined
    const arg =
      argCount > 1 ? parseEvaluationResultValue(args[1], handles) : undefined
    // biome-ignore lint/security/noGlobalEval: the agent's script, evaluated in the page's global scope as upstream's callback does.
    let result: unknown = globalThis.eval(expression)
    if (
      isFunction === true ||
      (isFunction !== false && typeof result === 'function')
    )
      result = (result as (element: unknown, arg: unknown) => unknown)(
        element,
        arg,
      )
    return result ? 'true' : 'false'
  }
  // The predicate as upstream's server writes it: the expression is evaluated once (a string
  // expression on every poll), the argument is parsed once, so a predicate that keeps state in
  // its argument sees it on the next poll, and the page judges truthiness. A truthy thenable
  // settles the wait with whatever it resolves to, as upstream's `fulfill(success)` does.
  const arg =
    argCount > 0 ? parseEvaluationResultValue(args[0], handles) : undefined
  let evaledExpression: unknown
  const predicate = (): unknown => {
    // biome-ignore lint/security/noGlobalEval: the agent's script, evaluated in the page's global scope as upstream's predicate does.
    let result: unknown = evaledExpression ?? globalThis.eval(expression)
    if (
      isFunction === true ||
      (isFunction !== false && typeof result === 'function')
    ) {
      evaledExpression = result
      result = (result as (arg: unknown) => unknown)(arg)
    }
    return result
  }
  const poll: Poller = () => {
    const success = predicate()
    if (!success) return [false]
    return new Promise<unknown>((fulfill) => fulfill(success)).then(
      (value): PollOutcome => {
        poll.last = value
        return [true, asText(utility.jsonValue(true, value))]
      },
    )
  }
  // Upstream runs the predicate's first poll inside the call that builds it; a throw here is
  // that call's exception.
  poll.first = poll()
  return poll
}
