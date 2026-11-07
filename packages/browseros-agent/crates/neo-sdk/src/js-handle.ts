/**
 * Playwright: `client/jsHandle.ts` `JSHandle`
 * Playwright: `server/javascript.ts` `normalizeEvaluationExpression`
 * Playwright: `server/dispatchers/jsHandleDispatcher.ts` `parseArgument`
 *
 * Playwright's client `JSHandle`, and the value chain Playwright's client and server run for
 * every evaluation, in the realm: Rust (neo-runtime `evaluate.rs`) passes values through as JSON
 * and maps handle guids to page objects, so every Playwright step that shapes a value runs here,
 * with Playwright's own vendored, unchanged serializers.
 *
 * ```text
 *  argument  client serializeValue (refuses functions)  → protocol value + JSHandles
 *            server parseSerializedValue                 → live value with handle markers
 *            server serializeAsCallArgument              → page value + guids, {h: i} in order
 *            → Rust → UtilityScript.evaluate in the frame's main world
 *  result    page jsonValue → Rust → parseEvaluationResultValue → serializeValue → parseResult
 * ```
 *
 * A handle carries the guid Rust minted (`handle@<n>`), never a CDP object id. Handles last one
 * run: Rust releases them all when the run ends.
 *
 * What differs from Playwright:
 * - `exposeFunctions: true` is refused: Neo cannot call back into the realm from the page yet.
 * - A `URL` crossing either way is Neo's stand-in (`vendor-shims.ts`): QuickJS has no WHATWG URL.
 * - An `ElementHandle`'s preview is final when the handle is created (Playwright updates it from
 *   `JSHandle@node` by event).
 */

import type { SerializedValue as ProtocolValue } from '@protocol/structs'
import {
  parseEvaluationResultValue,
  serializeAsCallArgument,
} from '../vendor/playwright/isomorphic/utilityScriptSerializers'
import {
  parseSerializedValue,
  serializeValue,
} from '../vendor/playwright/protocol/src/serializers'
import type { ApiName, Connection } from './connection'
import { isTargetClosedError } from './errors'

/**
 * An evaluation argument after Playwright's client and server steps (`serializeArgument`
 * below): `value` in the page's `UtilityScript` format, which Rust never reads, and the guids of
 * the handles its `{h: i}` references name, in order.
 */
export type SerializedArgument = { value: unknown; handles: string[] }

/** `Frame.evaluateExpression(Handle)`'s and `JSHandle.evaluateExpression(Handle)`'s params. */
export type EvaluateParams = {
  expression: string
  isFunction: boolean
  arg: SerializedArgument
}

/** Playwright's `JSHandleInitializer`. */
export type JSHandleInitializer = { preview: string }

/**
 * A handle Rust created: Playwright's `__create__` record for it. `type` is `ElementHandle` for
 * a DOM node, as Playwright's `JSHandleDispatcher` types it.
 */
export type CreatedHandle = {
  type: 'JSHandle' | 'ElementHandle'
  guid: string
  initializer: JSHandleInitializer
}

/** A by-value answer: the page's JSON, absent for `undefined`. */
export type EvaluateResult = { value?: unknown }

/** The evaluation commands on a frame guid, shaped as the generated `FrameCommands`. */
export type EvalFrameCommands = {
  evaluateExpression: { params: EvaluateParams; result: EvaluateResult }
  evaluateExpressionHandle: {
    params: EvaluateParams
    result: { handle: CreatedHandle }
  }
}

/** The `JSHandle` commands on a handle guid (`handle@<n>`). */
export type JSHandleCommands = EvalFrameCommands & {
  jsonValue: { params: Record<string, never>; result: EvaluateResult }
  dispose: { params: Record<string, never>; result: Record<string, never> }
}

/** Playwright's `Unboxed` (`types/structs.d.ts`): a handle in an argument arrives in the page
 * as the value it names. */
export type Unboxed<Arg> =
  Arg extends JSHandle<infer T>
    ? T
    : Arg extends object
      ? { [Key in keyof Arg]: Unboxed<Arg[Key]> }
      : Arg

/** Playwright's `PageFunction`: a function, or an expression string, evaluated in the page. */
export type PageFunction<Arg, R> =
  | string
  | ((arg: Unboxed<Arg>) => R | Promise<R>)

/** Playwright's `PageFunctionOn`: called with the handle's page value first. */
export type PageFunctionOn<On, Arg2, R> =
  | string
  | ((on: On, arg2: Unboxed<Arg2>) => R | Promise<R>)

/** A DOM element in the page. The realm has no DOM types, so the SDK cannot name its type. */
// biome-ignore lint/suspicious/noExplicitAny: page values are untyped in the realm
export type PageElement = any

/** Playwright's `EvaluateOptions` (`client/jsHandle.ts`). */
export type EvaluateOptions = { exposeFunctions?: boolean }

/** Stands for a `JSHandle` between Playwright's server parse and its call-argument step. */
class HandleRef {
  constructor(readonly guid: string) {}
}

export class JSHandle<T = unknown> {
  readonly _guid: string
  readonly _connection: Connection
  /** `jsHandle` or `elementHandle`: the prefix of this handle's errors. */
  readonly _apiPrefix: 'jsHandle' | 'elementHandle'
  _preview: string
  /** Phantom: the page value's type, as Playwright's `JSHandle<T>`. */
  declare readonly __type: T

  constructor(
    connection: Connection,
    guid: string,
    initializer: JSHandleInitializer,
    apiPrefix: 'jsHandle' | 'elementHandle' = 'jsHandle',
  ) {
    this._connection = connection
    this._guid = guid
    this._apiPrefix = apiPrefix
    this._preview = initializer.preview
  }

  async evaluate<R, Arg>(
    pageFunction: PageFunctionOn<T, Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions,
  ): Promise<R> {
    return await this._evaluate(
      `${this._apiPrefix}.evaluate` as ApiName,
      pageFunction,
      arg,
      options,
    )
  }

  /** `evaluate` under a caller's `apiName` (`locator.evaluate` runs it on its element). */
  async _evaluate<R>(
    apiName: ApiName,
    pageFunction: unknown,
    arg: unknown,
    options: EvaluateOptions | undefined,
  ): Promise<R> {
    assertEvaluateOptions(options)
    const params = evaluateParams(apiName, pageFunction, arg, options)
    const result = await this._connection.sendHandle(
      apiName,
      this._guid,
      'evaluateExpression',
      params,
      0,
    )
    return parseResult(result.value) as R
  }

  async evaluateHandle<R, Arg>(
    pageFunction: PageFunctionOn<T, Arg, R>,
    arg?: Arg,
    options?: EvaluateOptions,
  ): Promise<JSHandle<R>> {
    return await this._evaluateHandle(
      `${this._apiPrefix}.evaluateHandle` as ApiName,
      pageFunction,
      arg,
      options,
    )
  }

  /** `evaluateHandle` under a caller's `apiName`. */
  async _evaluateHandle<R>(
    apiName: ApiName,
    pageFunction: unknown,
    arg: unknown,
    options: EvaluateOptions | undefined,
  ): Promise<JSHandle<R>> {
    assertEvaluateOptions(options)
    const params = evaluateParams(apiName, pageFunction, arg, options)
    const result = await this._connection.sendHandle(
      apiName,
      this._guid,
      'evaluateExpressionHandle',
      params,
      0,
    )
    return handleFrom(this._connection, result.handle) as JSHandle<R>
  }

  async jsonValue(): Promise<T> {
    const result = await this._connection.sendHandle(
      `${this._apiPrefix}.jsonValue` as ApiName,
      this._guid,
      'jsonValue',
      {},
      0,
    )
    return parseResult(result.value) as T
  }

  asElement(): unknown {
    return null
  }

  async dispose(): Promise<void> {
    await this._dispose(`${this._apiPrefix}.dispose` as ApiName)
  }

  /** `dispose` under a caller's `apiName`. */
  async _dispose(apiName: ApiName): Promise<void> {
    try {
      await this._connection.sendHandle(apiName, this._guid, 'dispose', {}, 0)
    } catch (e) {
      if (isTargetClosedError(e)) return
      throw e
    }
  }

  toString(): string {
    return this._preview
  }
}

/** Builds the handle a `CreatedHandle` record names: an `ElementHandle` for a node. Set by
 * `element-handle.ts`, which extends this file's class (an import back would be a cycle). */
export const handleFactory: {
  element?: (
    connection: Connection,
    guid: string,
    initializer: JSHandleInitializer,
  ) => JSHandle
} = {}

/** The `JSHandle` or `ElementHandle` for one record Rust created. */
export function handleFrom(
  connection: Connection,
  created: CreatedHandle,
): JSHandle {
  if (created.type === 'ElementHandle' && handleFactory.element)
    return handleFactory.element(connection, created.guid, created.initializer)
  return new JSHandle(connection, created.guid, created.initializer)
}

/**
 * The params every evaluation sends: Playwright's client `{ expression: String(pageFunction),
 * isFunction: typeof pageFunction === 'function', arg }`, after its server's
 * `normalizeEvaluationExpression` and argument steps. A refusal throws `apiName`-prefixed, as the
 * server's errors read.
 */
export function evaluateParams(
  apiName: ApiName,
  pageFunction: unknown,
  arg: unknown,
  options: EvaluateOptions | undefined,
): EvaluateParams {
  // Deviation: `client/jsHandle.ts` `JSHandle.evaluate`: `const serializedArg = options?.exposeFunctions ? await serializeArgumentWithCallbacks(this, this._parentOfType('Page') as Page | undefined, arg) : serializeArgument(arg);` Neo has no realm callback from the page, so it refuses the option rather than ignore it.
  if (options?.exposeFunctions)
    throw new Error(`${apiName}: Neo does not support exposeFunctions yet`)
  const isFunction = typeof pageFunction === 'function'
  let expression: string
  try {
    expression = normalizeEvaluationExpression(String(pageFunction), isFunction)
  } catch (error) {
    throw new Error(`${apiName}: ${(error as Error).message}`)
  }
  return { expression, isFunction, arg: serializeArgument(arg) }
}

/**
 * Playwright's client `serializeArgument` (functions throw "Attempting to serialize unexpected
 * value") followed by its server's `parseArgument` and `serializeAsCallArgument`: the page-format
 * value and the guid of each handle reference, in reference order. This is the encoder every
 * `SerializedArgument` of a Neo call uses (`waitForFunction`'s, `dispatchEvent`'s `eventInit`).
 */
export function serializeArgument(arg: unknown): SerializedArgument {
  const channels: JSHandle[] = []
  const protocolValue: ProtocolValue = serializeValue(arg, (value) => {
    if (value instanceof JSHandle) return { h: channels.push(value) - 1 }
    return { fallThrough: value }
  })
  // Playwright: `server/dispatchers/jsHandleDispatcher.ts` `parseArgument`: `return parseSerializedValue(arg.value, arg.handles.map(a => (a as JSHandleDispatcher)._object));`
  const live = parseSerializedValue(
    protocolValue,
    channels.map((handle) => new HandleRef(handle._guid)),
  )
  const handles: string[] = []
  const value = serializeAsCallArgument(live, (value) => {
    if (value instanceof HandleRef) return { h: handles.push(value.guid) - 1 }
    return { fallThrough: value }
  })
  return { value, handles }
}

/**
 * The page's answer, as Playwright's server and client read it: `parseEvaluationResultValue`
 * (`crExecutionContext.ts`), the dispatcher's `serializeResult`, the client's `parseResult`.
 */
export function parseResult(value: unknown): unknown {
  const live = parseEvaluationResultValue(
    value as Parameters<typeof parseEvaluationResultValue>[0],
  )
  // Playwright: `server/dispatchers/jsHandleDispatcher.ts` `serializeResult`: `return serializeValue(arg, value => ({ fallThrough: value }));`
  const wire = serializeValue(live, (value) => ({ fallThrough: value }))
  // Playwright: `client/jsHandle.ts` `parseResult`: `return parseSerializedValue(value, undefined);`
  return parseSerializedValue(wire, undefined)
}

/** Playwright: `client/jsHandle.ts` `assertMaxArguments`: `throw new Error('Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.');` */
export function assertMaxArguments(count: number, max: number): void {
  if (count > max)
    throw new Error(
      'Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.',
    )
}

/** Playwright: `client/jsHandle.ts` `assertEvaluateOptions`: `throw new Error('Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.');` */
export function assertEvaluateOptions(options: unknown): void {
  if (
    options !== undefined &&
    (typeof options !== 'object' || options === null || Array.isArray(options))
  )
    throw new Error(
      'Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.',
    )
}

/** Playwright: `server/javascript.ts` `normalizeEvaluationExpression`: `throw new Error('Passed function is not well-serializable!');` */
export function normalizeEvaluationExpression(
  expression: string,
  isFunction: boolean | undefined,
): string {
  expression = expression.trim()

  if (isFunction) {
    try {
      new Function(`(${expression})`)
    } catch {
      // This means we might have a function shorthand. Try another
      // time prefixing 'function '.
      if (expression.startsWith('async '))
        expression = `async function ${expression.substring('async '.length)}`
      else expression = `function ${expression}`
      try {
        new Function(`(${expression})`)
      } catch {
        // We tried hard to serialize, but there's a weird beast here.
        throw new Error('Passed function is not well-serializable!')
      }
    }
  }

  if (/^(async)?\s*function(\s|\()/.test(expression))
    expression = `(${expression})`
  return expression
}
