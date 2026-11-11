/**
 * `page.addInitScript` and `context.addInitScript`: the script's source as upstream's client
 * builds it, the functions `exposeFunctions` passes it, and the disposable each returns. Rust
 * registers the source with CDP for the run (crates/neo-sdk/neo-playwright/src/page/
 * init_script.rs) and removes it when the run ends, so a script lasts as long as the agent's own
 * script, a later `goto` in it included.
 */
import {
  kBindingsControllerProperty,
  kFunctionBindingPrefix,
  serializeAsCallArgument,
} from '../../../neo-playwright/third_party/playwright/packages/isomorphic/utilityScriptSerializers'
import type { BindingCallback } from './bindings'
import { invoke } from './bridge'
import { type CallSite, captureSite } from './call-site'
import { plainError } from './errors'
import type { InitSource } from './wire'

const TOO_MANY_ARGUMENTS =
  'Too many arguments. If you need to pass more than 1 argument to the function wrap them in an object.'

/** Where an init script goes: a page, or the run's context. */
export type InitOwner = {
  /** Registers the source; the id `disposable.dispose` takes. */
  register(source: InitSource): Promise<number>
  /** Exposes `callback` under `name`, off `window` (upstream's `_exposeCallbackBinding`). */
  expose(name: string, callback: BindingCallback): Promise<Disposable>
}

/**
 * upstream's `addInitScript` on a page or the context: `assertEvaluateOptions`, then with
 * `exposeFunctions` the functions in `arg` become bindings the script calls
 * (`addInitScriptWithExposedFunctions`), else `evaluationScript`'s source.
 */
export async function addInitScript(
  api: string,
  owner: InitOwner,
  script: unknown,
  arg: unknown,
  options: unknown,
): Promise<Disposable> {
  if (
    options !== undefined &&
    (typeof options !== 'object' || options === null || Array.isArray(options))
  )
    throw plainError(api, TOO_MANY_ARGUMENTS)
  if (options && Reflect.get(options, 'exposeFunctions')) {
    if (typeof script !== 'function')
      throw plainError(
        api,
        'Passing functions requires the init script to be a function',
      )
    return await withExposedFunctions(owner, script, arg)
  }
  return registration(await owner.register(initSource(api, script, arg)))
}

/**
 * upstream's `initScriptSourceWithExposedFunctions`: each function in `arg` is exposed under a
 * fresh `__pw_fn_` name and serialized as `{ fn: name }`, which the page's controller turns back
 * into a function that calls the binding (`parseInitScriptArg`). The bindings go in before the
 * script, so every document runs their stubs first. Disposing removes the script and the
 * bindings, once.
 */
async function withExposedFunctions(
  owner: InitOwner,
  script: unknown,
  arg: unknown,
): Promise<Disposable> {
  const exposing: Promise<Disposable>[] = []
  const undo = async (error: unknown): Promise<never> => {
    // A failure part way (a refused tab, a duplicate name) leaves nothing behind: every
    // binding that went in is disposed before the call rejects.
    const settled = await Promise.allSettled(exposing)
    await Promise.all(
      settled.map((outcome) =>
        outcome.status === 'fulfilled'
          ? outcome.value.dispose().catch(() => {})
          : undefined,
      ),
    )
    throw error
  }
  let serialized: unknown
  try {
    serialized = serializeAsCallArgument(arg, (value: unknown) => {
      if (typeof value === 'function') {
        const name = callbackName()
        exposing.push(
          owner.expose(name, (_source, ...args) =>
            Reflect.apply(value, undefined, args),
          ),
        )
        return { fn: name }
      }
      return { fallThrough: value }
    })
  } catch (error) {
    return await undo(error)
  }
  let bindings: Disposable[]
  let init: Disposable
  try {
    bindings = await Promise.all(exposing)
    const source = `(${String(script)})(globalThis['${kBindingsControllerProperty}'].parseInitScriptArg(${JSON.stringify(serialized)}))`
    init = registration(await owner.register({ source }))
  } catch (error) {
    return await undo(error)
  }
  let disposed = false
  return new Disposable(async (site) => {
    if (disposed) return
    disposed = true
    await Promise.all(
      bindings.map((binding) => binding._dispose(site).catch(() => {})),
    )
    await init._dispose(site)
  })
}

/**
 * A fresh name for a function `exposeFunctions` passes, `addInitScript`'s or `evaluate`'s:
 * upstream's `kFunctionBindingPrefix` and `createGuid` (16 random bytes as hex). The page's
 * controller reads a `{ fn }` with this prefix as a call of that binding.
 */
export function callbackName(): string {
  let text = kFunctionBindingPrefix
  for (let index = 0; index < 16; index++)
    text += Math.floor(Math.random() * 256)
      .toString(16)
      .padStart(2, '0')
  return text
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
 * upstream's `DisposableObject` and `DisposableStub`, which `addInitScript` and `exposeBinding`
 * resolve to: `dispose()` removes what the call registered. Rust removes it at the run's end in
 * any case.
 */
export class Disposable {
  readonly #dispose: (site: CallSite) => Promise<void>

  constructor(dispose: (site: CallSite) => Promise<void>) {
    this.#dispose = dispose
  }

  async dispose(): Promise<void> {
    const site = captureSite()
    await this.#dispose(site)
  }

  /** Composite cleanup keeps its public caller's site after awaiting earlier disposals. */
  async _dispose(site: CallSite): Promise<void> {
    await this.#dispose(site)
  }
}

/**
 * The disposable of one registration Rust keeps under `id`. Disposing again asks Rust again,
 * which disposes nothing more, as upstream's server ignores a second dispose; `forget` (a
 * binding's callback leaving the realm) runs either way.
 */
export function registration(id: number, forget?: () => void): Disposable {
  return new Disposable(async (site) => {
    try {
      await invoke(
        'disposable.dispose',
        site,
        { api: 'disposable.dispose', timeoutMs: 0 },
        { id },
      )
    } finally {
      forget?.()
    }
  })
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
