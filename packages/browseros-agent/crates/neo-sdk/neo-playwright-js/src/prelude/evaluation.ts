/**
 * One main-world script's wire form, built as upstream's client and server build it before the
 * page, and its result read back as upstream's client reads it.
 *
 *   agent's arg ─ client serializeValue (its checks and errors) ─ the channel's validator
 *               ─ server parseSerializedValue ─ serializeAsCallArgument (the page's format)
 *               ─ JSON text on the wire
 *   result      ─ Rust's masked value in the page's format ─ parseEvaluationResultValue
 *
 * Neo's client and the server's value work both run here, in QuickJS, with the vendored
 * modules, so the page receives exactly what upstream's page receives and the agent gets
 * upstream's values back. Rust passes the argument on as text, and the host sees one string.
 *
 * Where upstream raises a check decides whether its message carries the `<api>:` prefix: the
 * client's throws come before its channel call adds one (`plainError`), and the server's
 * (normalization) come back through it. A server check is sent as `{ refused }`, and Rust raises
 * it at the server's point, after the page is checked, with nothing sent. Which client checks a
 * call runs, and whether before or after it finds its element, is its form (`FORMS`).
 *
 * Element handles pass as upstream's do: the client writes `{h: i}`, the server reads them back
 * as handles, and the page's format names each again by its place in the handles Rust resolves
 * in the main world (`run.handles`). Rust adopts a handle only into its own document.
 */
import {
  parseEvaluationResultValue,
  serializeAsCallArgument,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers'
import {
  parseSerializedValue,
  serializeValue,
} from '../../../neo-playwright/third_party/playwright/packages/protocol/src/serializers'
import { ElementHandle } from './element-handle'
import {
  answerError,
  clientError,
  plainError,
  unsupportedError,
} from './errors'
import { WaitHandle } from './frame'
import { callbackName } from './init-script'
import { isHandle } from './options'
import type { ElementBrand, PageScript, SerializedValue } from './wire'

/**
 * `evaluate`'s options: upstream's one. With `exposeFunctions`, each function in the argument
 * reaches the page as one it can call back (`Callbacks`).
 */
export type EvaluateOptions = { exposeFunctions?: boolean }

/**
 * The functions an argument passes with `exposeFunctions`, each under the fresh `__pw_fn_` name
 * its `{ fn: name }` carries to the page, as upstream's `serializeArgumentWithCallbacks` names
 * them. The caller exposes each as a binding of the script's page before the op goes out
 * (`Page._exposeEvaluateCallbacks`); the page's controller turns `{ fn }` into a function that
 * calls it.
 */
export type Callbacks = [
  name: string,
  callback: (...args: unknown[]) => unknown,
][]

/** Whether `options` asks for upstream's `exposeFunctions`. */
export function exposesFunctions(options: unknown): boolean {
  return (
    typeof options === 'object' &&
    options !== null &&
    !!Reflect.get(options, 'exposeFunctions')
  )
}

const TOO_MANY_ARGUMENTS =
  'Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.'

/** How upstream words a failed check: which refusal helper says it as upstream's client does. */
type CheckKind = 'unsupported' | 'client' | 'plain'

const REFUSALS: Record<CheckKind, (api: string, message: string) => Error> = {
  unsupported: unsupportedError,
  client: clientError,
  plain: plainError,
}

/**
 * A check of the script's call that failed, before anything reports it: `pageCall` does, once.
 * The frame, handle and query forms throw the door's refusal of it at once. locator.evaluate's
 * checks run after its wait upstream, so its op carries the message to Rust as `{ refused }`,
 * which fails that op: the op is the failure's one record, and no refusal is built for it,
 * since the door records every refusal it is asked for, thrown or not (FR/RC note 1).
 */
class FailedCheck extends Error {
  constructor(
    readonly kind: CheckKind,
    message: string,
  ) {
    super(message)
  }
}

/**
 * upstream's `assertMaxArguments` (PW:59239); callers pass `arguments.length`, as upstream, and
 * `api`, the method the refusal is recorded under.
 */
export function assertMaxArguments(
  api: string,
  count: number,
  max: number,
): void {
  if (count > max) throw plainError(api, TOO_MANY_ARGUMENTS)
}

/** upstream's `assertEvaluateOptions` (PW:59243): an options argument must be an object. */
function assertEvaluateOptions(options: unknown): void {
  if (
    options !== undefined &&
    (typeof options !== 'object' || options === null || Array.isArray(options))
  )
    throw new FailedCheck('plain', TOO_MANY_ARGUMENTS)
}

const fallThrough = (value: unknown) => ({ fallThrough: value })

/**
 * Where upstream raises each client check, per form of call:
 *
 * | form    | options check | argument checks | who                                        |
 * |---------|---------------|-----------------|--------------------------------------------|
 * | frame   | now           | now             | page/frame.evaluate, waitForFunction       |
 * | locator | after wait    | after wait      | locator.evaluate                           |
 * | handle  | now           | now             | elementHandle.evaluate                     |
 * | query   | none          | now             | $eval, $$eval, evaluateAll, the handle ones |
 * |         |               |                 | and locator.waitForFunction                 |
 *
 * "now" throws before any op, unprefixed for the client's own checks and prefixed for the
 * channel validator's. `locator.evaluate` defers everything: `_withElement` runs the handle's
 * evaluate inside its `_wrapApiCall`, after `waitForSelector` (PW:59823), so every check fails
 * after the wait with the prefix, as a refusal Rust raises there. Normalization is always the
 * server's, after the element is found. With `exposeFunctions`, `callbacks` collects the
 * argument's functions, which the caller exposes before its op.
 */
export type CallForm = 'frame' | 'locator' | 'handle' | 'query'

const FORMS: Record<CallForm, { options: boolean; afterWait: boolean }> = {
  frame: { options: true, afterWait: false },
  locator: { options: true, afterWait: true },
  handle: { options: true, afterWait: false },
  query: { options: false, afterWait: false },
}

/**
 * The script for `pageFunction(subject?, arg)` in `form`. The order is upstream's: the options,
 * then the client serializes the argument (throwing for a function inside it), the channel's
 * validator checks it and its handles, then the server normalizes the expression and
 * serializes the argument again in the page's format.
 */
export function pageCall(
  api: string,
  form: CallForm,
  pageFunction: unknown,
  arg: unknown,
  options?: unknown,
  callbacks?: Callbacks,
): PageScript {
  const { options: checksOptions, afterWait } = FORMS[form]
  try {
    if (checksOptions) assertEvaluateOptions(options)
    return script(api, pageFunction, arg, callbacks)
  } catch (error) {
    if (afterWait) {
      const message = error instanceof Error ? error.message : String(error)
      return {
        refused: message.startsWith(`${api}: `)
          ? message.slice(api.length + 2)
          : message,
      }
    }
    if (error instanceof FailedCheck)
      throw REFUSALS[error.kind](api, error.message)
    throw error
  }
}

/** The script once the options passed: the argument's checks, normalization, the page form. */
function script(
  api: string,
  pageFunction: unknown,
  arg: unknown,
  callbacks: Callbacks | undefined,
): PageScript {
  const isFunction = typeof pageFunction === 'function'
  const { serialized, clientHandles } = serializeArgument(api, arg, callbacks)
  let expression: string
  try {
    expression = normalizeEvaluationExpression(String(pageFunction), isFunction)
  } catch (error) {
    return { refused: error instanceof Error ? error.message : String(error) }
  }
  const onServer: unknown = parseSerializedValue(serialized, clientHandles)
  const handles: ElementBrand[] = []
  const text = JSON.stringify(
    serializeAsCallArgument(onServer, (value) => {
      if (!(value instanceof ElementHandle)) return fallThrough(value)
      handles.push(value._brand)
      return { h: handles.length - 1 }
    }),
  )
  return {
    run: {
      expression,
      isFunction,
      arg: text,
      ...(handles.length > 0 ? { handles } : {}),
    },
  }
}

/**
 * upstream's client `serializeArgument`: an element handle becomes `{h: i}`, a
 * `waitForFunction` handle passes its value (a copy: Neo keeps no remote object past the wait),
 * Neo's other objects are refused, and its errors are unprefixed. Then the channel's
 * validator: the value, and each handle's object, which a disposed handle no longer has.
 *
 * A handle the argument names twice passes one copy, which the serializer then refers back to,
 * so the page receives one object for it, as upstream's page receives its one remote object.
 */
function serializeArgument(
  api: string,
  arg: unknown,
  callbacks: Callbacks | undefined,
) {
  let serialized: ReturnType<typeof serializeValue>
  const clientHandles: ElementHandle[] = []
  const copies = new Map<WaitHandle, unknown>()
  try {
    serialized = serializeValue(arg, (value) => {
      // upstream's `registerCallback`: without `exposeFunctions` a function falls through to
      // the serializer's own "Attempting to serialize unexpected value".
      if (typeof value === 'function' && callbacks) {
        const name = callbackName()
        callbacks.push([name, value as Callbacks[number][1]])
        return { fn: name }
      }
      if (value instanceof ElementHandle) {
        clientHandles.push(value)
        return { h: clientHandles.length - 1 }
      }
      if (value instanceof WaitHandle) {
        if (!copies.has(value)) copies.set(value, value._value(api))
        return fallThrough(copies.get(value))
      }
      if (!isHandle(value)) return fallThrough(value)
      throw new FailedCheck(
        'unsupported',
        'Neo passes element handles as evaluate arguments, not pages, frames, locators or JSHandles; pass the value itself, or an element handle',
      )
    })
  } catch (error) {
    if (error instanceof FailedCheck) throw error
    throw new FailedCheck(
      'plain',
      error instanceof Error ? error.message : String(error),
    )
  }
  validateValue(serialized, 'arg.value')
  clientHandles.forEach((handle, index) => {
    if (handle._disposed)
      throw new FailedCheck(
        'client',
        `arg.handles[${index}]: no object with guid ${handle._guid}`,
      )
  })
  return { serialized, clientHandles }
}

/**
 * upstream's channel validator on the serialized argument (`scheme.SerializedValue`,
 * validatorPrimitives.ts), for the fields the client's serializer can write with another type
 * than the protocol's: an invalid Date's `toJSON` is null, and an Error's message, name or stack
 * can be anything. It walks in the scheme's order (`d`, then `e`'s `m`, `n`, `s`, then `a` and
 * `o` depth first) and fails inside the channel call, so its message carries the prefix.
 */
function validateValue(
  value: ReturnType<typeof serializeValue>,
  path: string,
): void {
  const expectString = (field: unknown, at: string) => {
    if (typeof field !== 'string' && !(field instanceof String))
      throw new FailedCheck(
        'client',
        `${at}: expected string, got ${typeof field}`,
      )
  }
  if (value.d !== undefined) expectString(value.d, `${path}.d`)
  if (value.e !== undefined)
    for (const key of ['m', 'n', 's'] as const)
      expectString(value.e[key], `${path}.e.${key}`)
  value.a?.forEach((item, index) => {
    validateValue(item, `${path}.a[${index}]`)
  })
  value.o?.forEach((entry, index) => {
    validateValue(entry.v, `${path}.o[${index}].v`)
  })
}

/**
 * `value` as a page receives an argument, JSON text in Playwright's in-page format, for what Neo
 * hands a page outside an evaluate: a binding call's answer, which upstream's client serializes
 * (`serializeArgument`) and its server re-serializes for the page. A value the protocol cannot
 * carry throws its plain error, and so does a handle: a binding answers with values only.
 */
export function inPageArgument(value: unknown): string {
  const serialized = serializeValue(value, (item) => {
    if (isHandle(item) || item instanceof WaitHandle)
      throw new Error(
        'Neo answers a binding with values, not handles; return the value itself',
      )
    return fallThrough(item)
  })
  return JSON.stringify(
    serializeAsCallArgument(parseSerializedValue(serialized, []), fallThrough),
  )
}

/** One argument a page sent in its own format (a binding call's), as the agent's value. */
export function fromInPageArgument(value: unknown): unknown {
  return parseEvaluationResultValue(value as never)
}

/**
 * The op's masked result, in the page's format, as the agent's value: the vendored
 * `parseEvaluationResultValue`. Rust already applied upstream's server-to-client step (an
 * ArrayBuffer is `{}`), so this one walk gives what upstream's three do.
 *
 * The walk recurses once per level of the value, and claw-sdk's QuickJS stack is far smaller
 * than Node's, so a result nested as deep as Chromium carries one overflows it where
 * upstream's client reads it (tier 1 `evaluate-errors`, `deepest`). The op ran and answered,
 * and reading its answer failed, so the failure is the op's, named for `api`, as it was when
 * claw-sdk's bootstrap revived the answer and overflowed inside the op.
 */
export function fromPageValue(api: string, value: SerializedValue): unknown {
  try {
    return parseEvaluationResultValue(value as never)
  } catch (error) {
    throw answerError(api, error)
  }
}

/*
 * upstream's server `normalizeEvaluationExpression` (packages/playwright-core/src/server/
 * javascript.ts at v1.64.1, PW:20638). Copyright (c) Microsoft Corporation, Apache License 2.0.
 * It runs in QuickJS, whose parser already parsed the agent's function as part of its script,
 * so `new Function` accepts what V8 would.
 */
function normalizeEvaluationExpression(
  expression: string,
  isFunction: boolean | undefined,
): string {
  expression = expression.trim()
  if (isFunction) {
    try {
      new Function(`(${expression})`)
    } catch {
      if (expression.startsWith('async '))
        expression = `async function ${expression.substring('async '.length)}`
      else expression = `function ${expression}`
      try {
        new Function(`(${expression})`)
      } catch {
        throw new Error('Passed function is not well-serializable!')
      }
    }
  }
  if (/^(async)?\s*function(\s|\()/.test(expression))
    expression = `(${expression})`
  return expression
}
