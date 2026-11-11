/**
 * `page.addInitScript` and `context.addInitScript`: the script's source as upstream's client
 * builds it, and the disposable each returns. Rust registers the source with CDP for the run
 * (crates/neo-sdk/neo-playwright/src/page/init_script.rs) and removes it when the run ends, so a
 * script lasts as long as the agent's own script, a later `goto` in it included.
 */
import { invoke } from './bridge'
import { plainError, unsupportedError } from './errors'
import type { InitSource } from './wire'

const TOO_MANY_ARGUMENTS =
  'Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.'

/**
 * upstream's client checks of `addInitScript`'s options, in its order: `assertEvaluateOptions`,
 * then `exposeFunctions`, which needs a function. Neo does not serve `exposeFunctions` yet: the
 * page would call back into the agent's realm, which needs bindings.
 */
export function checkInitOptions(
  api: string,
  script: unknown,
  options: unknown,
): void {
  if (
    options !== undefined &&
    (typeof options !== 'object' || options === null || Array.isArray(options))
  )
    throw plainError(api, TOO_MANY_ARGUMENTS)
  if (!options || !Reflect.get(options, 'exposeFunctions')) return
  if (typeof script !== 'function')
    throw plainError(
      api,
      'Passing functions requires the init script to be a function',
    )
  throw unsupportedError(
    api,
    'pages cannot call back into the script in Neo; pass values the page can use instead',
  )
}

/**
 * upstream's client `evaluationScript` (clientHelper.ts): a function becomes a call with its
 * argument spelled by `JSON.stringify`, which drops functions and `undefined` members as
 * upstream's does; a string or `{ content }` is the source as given; `{ path }` goes to Rust,
 * which reads the file from Neo's disk as upstream's client reads it from its own. The checks
 * throw before any op, unprefixed, as upstream's client throws outside its channel call.
 */
export function initSource(
  api: string,
  script: unknown,
  arg: unknown,
): InitSource {
  if (typeof script === 'function') {
    const argText = Object.is(arg, undefined)
      ? 'undefined'
      : JSON.stringify(arg)
    return { source: `(${String(script)})(${argText})` }
  }
  if (arg !== undefined)
    throw plainError(api, 'Cannot evaluate a string with arguments')
  if (typeof script === 'string') return { source: script }
  if (typeof script === 'object' && script !== null) {
    const content: unknown = Reflect.get(script, 'content')
    if (content !== undefined) return { source: String(content) }
    const path: unknown = Reflect.get(script, 'path')
    if (path !== undefined) return { path: String(path) }
  }
  throw plainError(api, 'Either path or content property must be present')
}

/**
 * upstream's `DisposableObject`, which `addInitScript` resolves to: `dispose()` removes what the
 * call registered, and disposing again does nothing, as upstream's server ignores a second
 * dispose. Rust removes it at the run's end in any case.
 */
export class Disposable {
  readonly #id: number

  constructor(id: number) {
    this.#id = id
  }

  async dispose(): Promise<void> {
    await invoke(
      'disposable.dispose',
      { api: 'disposable.dispose', timeoutMs: 0 },
      { id: this.#id },
    )
  }
}

// `await using` support where the realm has the symbol; QuickJS may not, and the prelude defines
// no global, so it is added only when it exists.
const asyncDispose: unknown = Reflect.get(Symbol, 'asyncDispose')
if (typeof asyncDispose === 'symbol')
  Object.defineProperty(Disposable.prototype, asyncDispose, {
    configurable: true,
    writable: true,
    value(this: Disposable) {
      return this.dispose()
    },
  })
