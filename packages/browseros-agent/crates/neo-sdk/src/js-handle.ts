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
import { type ApiName, type Connection, rebuildError } from './connection'
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

/** The selector commands a frame and an element handle share (Playwright's `$`, `$$`, `$eval`,
 * `$$eval`), with the evaluation's `arg` in the page's format. */
export type SelectorCommands = {
  querySelector: {
    params: { selector: string; strict?: boolean }
    result: { element?: CreatedHandle }
  }
  querySelectorAll: {
    params: { selector: string }
    result: { elements: CreatedHandle[] }
  }
  evalOnSelector: {
    params: { selector: string; strict?: boolean } & EvaluateParams
    result: EvaluateResult
  }
  evalOnSelectorAll: {
    params: { selector: string } & EvaluateParams
    result: EvaluateResult
  }
}

/** The evaluation commands on a frame guid, shaped as the generated `FrameCommands`. */
export type EvalFrameCommands = SelectorCommands & {
  evaluateExpression: { params: EvaluateParams; result: EvaluateResult }
  evaluateExpressionHandle: {
    params: EvaluateParams
    result: { handle: CreatedHandle }
  }
  waitForFunction: {
    params: EvaluateParams & { pollingInterval?: number }
    result: { handle: CreatedHandle }
  }
}

/** The `JSHandle` commands on a handle guid (`handle@<n>`); the selector ones are an
 * `ElementHandle`'s. */
export type JSHandleCommands = EvalFrameCommands & {
  jsonValue: { params: Record<string, never>; result: EvaluateResult }
  dispose: { params: Record<string, never>; result: Record<string, never> }
  getProperty: { params: { name: string }; result: { handle: CreatedHandle } }
  getPropertyList: {
    params: Record<string, never>
    result: { properties: { name: string; value: CreatedHandle }[] }
  }
}

/** Playwright's `Unboxed`: a handle in an argument arrives in the page as the value it names. */
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

/** Playwright's `EvaluateOptions`. */
export type EvaluateOptions = { exposeFunctions?: boolean }

/** Neo: stands for a `JSHandle` between Playwright's server parse and its call-argument step,
 * where Playwright's server holds its own `JSHandle` object. */
class HandleRef {
  constructor(readonly guid: string) {}
}

/** Playwright: `client/jsHandle.ts` `JSHandle` */
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

  async getProperty(propertyName: string): Promise<JSHandle> {
    const result = await this._connection.sendHandle(
      `${this._apiPrefix}.getProperty` as ApiName,
      this._guid,
      'getProperty',
      { name: propertyName },
      0,
    )
    return handleFrom(this._connection, result.handle)
  }

  async getProperties(): Promise<Map<string, JSHandle>> {
    const map = new Map<string, JSHandle>()
    const result = await this._connection.sendHandle(
      `${this._apiPrefix}.getProperties` as ApiName,
      this._guid,
      'getPropertyList',
      {},
      0,
    )
    for (const { name, value } of result.properties)
      map.set(name, handleFrom(this._connection, value))
    return map
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

/** Neo: builds the handle a `CreatedHandle` record names: an `ElementHandle` for a node. Set by
 * `element-handle.ts`, which extends this file's class (an import back would be a cycle). */
export const handleFactory: {
  element?: (
    connection: Connection,
    guid: string,
    initializer: JSHandleInitializer,
  ) => JSHandle
} = {}

/** Neo: the `JSHandle` or `ElementHandle` for one record Rust created, where Playwright's
 * connection builds the object from its `__create__` message. */
export function handleFrom(
  connection: Connection,
  created: CreatedHandle,
): JSHandle {
  if (created.type === 'ElementHandle' && handleFactory.element)
    return handleFactory.element(connection, created.guid, created.initializer)
  return new JSHandle(connection, created.guid, created.initializer)
}

/**
 * Playwright: `client/jsHandle.ts` `JSHandle.evaluate`: `{ expression: String(pageFunction), isFunction: typeof pageFunction === 'function', arg: serializedArg }`
 *
 * The params every evaluation sends, after Playwright's server steps
 * (`normalizeEvaluationExpression`, the argument chain). A refusal throws `apiName`-prefixed, as
 * the server's errors read.
 */
export function evaluateParams(
  apiName: ApiName,
  pageFunction: unknown,
  arg: unknown,
  options: EvaluateOptions | undefined,
): EvaluateParams {
  if (options?.exposeFunctions) {
    // Deviation: `client/jsHandle.ts` `JSHandle.evaluate`: `const serializedArg = options?.exposeFunctions ? await serializeArgumentWithCallbacks(this, this._parentOfType('Page') as Page | undefined, arg) : serializeArgument(arg);` Neo has no realm callback from the page, so it refuses the option rather than ignore it.
    throw new Error(`${apiName}: Neo does not support exposeFunctions yet`)
  }
  const isFunction = typeof pageFunction === 'function'
  let expression: string
  try {
    expression = normalizeEvaluationExpression(String(pageFunction), isFunction)
  } catch (error) {
    // Playwright's server error, rebuilt as its client rebuilds every server error.
    throw rebuildError(
      apiName,
      { name: 'Error', message: (error as Error).message },
      undefined,
    )
  }
  return { expression, isFunction, arg: serializeArgument(arg) }
}

/**
 * Playwright: `client/jsHandle.ts` `serializeArgument`
 *
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
 * Playwright: `client/jsHandle.ts` `parseResult`
 *
 * The page's answer, as Playwright's server and client read it: `parseEvaluationResultValue`
 * (Chromium's execution context), the dispatcher's `serializeResult`, the client's `parseResult`.
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
