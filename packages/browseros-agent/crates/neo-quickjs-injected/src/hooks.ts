/**
 * Neo: the hook realm's entry, `createHooks`. A run that carries a hook file gets a second QuickJS
 * realm next to the agent's (neo-runtime `hooks.rs`). Rust evaluates this same bundle there, calls
 * `createHooks`, runs the hook file as the body of an async function of `neo`, then calls
 * `finishLoading` once and `invoke` once per hook event it fires, one at a time.
 *
 * ```text
 *  load:   body(neo) → neo.on(event, [filter], handler) …   finishLoading() → '[{"event":…},…]'
 *  fire:   invoke(id, 'beforeRun', '{"script":…,"page":"page@T","context":"context@default"}')
 *            → event (this realm's real Page, BrowserContext …) and ctx
 *            → matching handlers in registration order, returns folded
 *            → '{"decision":"allow"}' | '{"decision":"block","reason":…}' | … | '{"decision":"fail",…}'
 *  budget: abort(id, reason) → that invocation's ctx.signal aborts
 * ```
 *
 * - `invoke` never rejects. A handler that throws or rejects, a return outside its event's
 *   allowed shapes, a payload it cannot read or a replacement it cannot export is a `fail`
 *   decision, which Rust applies fail closed (design D8). Its `error` text is for the host-only
 *   hook trace, never the model: a hook's error may hold a secret.
 * - Rust owns each invocation's budget and the run's cancellation; this file only turns `abort`
 *   into `ctx.signal`. Once the signal aborts no further handler of that invocation starts, but
 *   one already running keeps going until it settles; Rust has stopped waiting for it by then.
 * - The calls this realm's objects send are hook calls because Rust's hook bridge received them,
 *   never because of a field in the message: the agent can forge any field it sends.
 * - Hook listeners never fire: the hook bridge produces no events (its `subscribe` is a no-op).
 */

import { BrowserContext, pageGuidOf } from './browser-context'
import { exportValue } from './buffer'
import { ElementHandle } from './element-handle'
import type { Frame } from './frame'
import { standardGlobals } from './globals'
import type { Locator } from './locator'
import type { Page } from './page'
import { connectRun, type RunInit, type RunObjects } from './run-objects'
import { get, log } from './runtime-bridge'

/** What neo-runtime hands `createHooks`: `createRun`'s init plus the run's `meta`. */
export interface HooksInit extends RunInit {
  /** The run's meta, already parsed JSON; `null` or absent when the host sent none. */
  meta: unknown
}

/** A hook event (design D1): the run's start and end, and each browser call's start and end. */
export type HookEvent =
  | 'beforeRun'
  | 'afterRun'
  | 'beforeAction'
  | 'afterAction'

/** The actions an action-event registration wants: kind in `kinds` OR group in `groups`. */
export interface HookFilter {
  kinds?: readonly string[]
  groups?: readonly string[]
}

/** A handler's second argument. One per invocation, shared by that invocation's handlers. */
export interface HookContext {
  /** The run's meta: a deep-frozen copy, `null` when the host sent none. */
  readonly meta: unknown
  /** One plain object per run, shared by every handler of every event. */
  readonly state: Record<string, unknown>
  /** Aborts when Rust calls `abort` for this invocation: its budget expired or the run was
   * cancelled. */
  readonly signal: AbortSignal
  /** Appends one line to the run's hook trace, which only the host reads. */
  trace(...values: unknown[]): void
  /** Asks whoever started the run (design D9, F5). Until F5 it always rejects with an `Error`. */
  host(name: string, args?: unknown): Promise<unknown>
}

/** A handler: `event` is built for each handler from Rust's payload, `ctx` for each invocation.
 * What it returns, or resolves with, is its decision. */
export type HookHandler = (
  event: Record<string, unknown>,
  ctx: HookContext,
) => unknown

/** The `neo` a hook file's body receives. */
export interface NeoHooks {
  /** Registers `handler` for `event`, while the hook file loads; a `TypeError` otherwise. A
   * `filter` is allowed only on the action events. */
  on(event: HookEvent, handler: HookHandler): void
  on(
    event: HookEvent,
    filter: HookFilter | null | undefined,
    handler: HookHandler,
  ): void
}

/** neo-runtime's native `ship` (`quickjs.rs` `to_json`): copies a binary's bytes out of the
 * realm and answers its index. */
export type Ship = (bytes: Uint8Array) => number

/** What `createHooks` gives neo-runtime. Every function ignores `this`, so Rust may call it
 * detached from the object. */
export interface Hooks {
  /** Passed to the hook file's body as `neo`. */
  neo: NeoHooks
  /** The standard globals neo-runtime defines before the hook file runs, as for `createRun`. */
  globals: Record<string, unknown>
  /** `buffer.ts` `exportValue`, as `createRun` gives it. `invoke` already exports each `replace`
   * value with it; neo-runtime need not call it. */
  exportValue: typeof exportValue
  /** Rejects with the first listener error nobody handled in this realm; never resolves. An
   * invocation running then, and every later one, decides `fail`. */
  uncaught: Promise<never>
  /** Ends registration and returns it as JSON, `[{event, kinds?, groups?}]` in registration
   * order, so Rust can skip the realm for actions nothing wants. */
  finishLoading(): string
  /**
   * Runs `event`'s matching handlers on `payload` (contract §3) and resolves with the decision
   * JSON (§4); never rejects. With neo-runtime's `ship` and a fresh `marker`, a `replace`
   * decision's `value` is `exportValue`'s JSON, each binary marked with its shipped index for
   * `quickjs.rs` `lift_binaries`; without them, it is the realm's `JSON.stringify`. A `replace`
   * whose value JSON drops (`undefined`) has no `value` key.
   */
  invoke(
    id: number,
    event: string,
    payload: string,
    ship?: Ship,
    marker?: string,
  ): Promise<string>
  /** Aborts invocation `id`'s `ctx.signal` with an `AbortError` carrying `reason`; an id with
   * no handler running is ignored. */
  abort(id: number, reason: string): void
}

/**
 * Neo: the events a hook file registers for and Rust fires (design D1). Any other name is a
 * `TypeError` at `neo.on` and a `fail` decision at `invoke`.
 */
const HOOK_EVENTS: readonly HookEvent[] = [
  'beforeRun',
  'afterRun',
  'beforeAction',
  'afterAction',
]

/** The realm's `JSON.stringify` and `JSON.parse` as the bundle loads, before a hook file could
 * replace them. */
const stringify = JSON.stringify
const parse = JSON.parse

/**
 * Builds the hook realm's objects as `createRun` builds the agent's (`run-objects.ts`), and the
 * `neo`, `finishLoading`, `invoke` and `abort` neo-runtime drives the hook file with. Reads and
 * sends nothing until a handler does.
 */
export function createHooks(init: HooksInit): Hooks {
  const realm = new HookRealm(init)
  return {
    neo: Object.freeze({
      on: (event: unknown, second?: unknown, third?: unknown) =>
        realm.on(event, second, third),
    }),
    globals: standardGlobals(),
    exportValue,
    uncaught: realm.uncaught,
    finishLoading: () => realm.finishLoading(),
    invoke: (id, event, payload, ship, marker) =>
      realm.invoke(id, event, payload, ship, marker),
    abort: (id, reason) => realm.abort(id, reason),
  }
}

/** One `neo.on` call: fixed once made, so Rust's copy of the filters stays true. */
interface Registration {
  readonly event: HookEvent
  readonly kinds?: readonly string[]
  readonly groups?: readonly string[]
  readonly handler: HookHandler
  /** 1-based, among every registration of the file: what the hook trace names it by. */
  readonly index: number
}

/** An invocation whose handlers may still run. */
interface Invocation {
  readonly controller: AbortController
  /** Decides the invocation now; a no-op once it is decided. */
  readonly settle: (decision: string) => void
}

/** The real objects an invocation's payload names, built once and shared by its handlers. */
interface EventObjects {
  context: BrowserContext
  page?: Page
  frame?: Frame
  locator?: Locator
  /** An element action's pinned element (F4): a handle only this realm can use. */
  element?: ElementHandle
}

/** A payload JSON object (contract §3). Guids and `kind`/`group` are checked where read; every
 * other field reaches handlers as Rust sent it. */
type Payload = Record<string, unknown>

/** A before-event handler's checked return. */
type Before = { allow: true } | { block: string } | { replaceArgs: string }

/** An after-event handler's checked return: `replace` present means a replacement, `undefined`
 * included. */
type After = { replace?: { value: unknown }; note?: string }

/** The state behind one `createHooks`: per realm, so per run. */
class HookRealm {
  readonly uncaught: Promise<never>
  readonly #objects: RunObjects
  readonly #meta: unknown
  readonly #state: Record<string, unknown> = {}
  readonly #registrations: Registration[] = []
  #loading = true
  /** By id, the invocations whose handlers may still run: `abort` and an uncaught error reach
   * them. */
  readonly #live = new Map<number, Invocation>()
  /** Set once a listener error went unhandled: the realm decides `fail` from then on. */
  #broken: { error: unknown } | undefined
  /** By guid, the contexts payloads named: the run's, and any other built on first sight. */
  readonly #contexts = new Map<string, BrowserContext>()
  readonly #trace = (...values: unknown[]): void => {
    log(values.map(describe).join(' '))
  }
  readonly #host = async (): Promise<unknown> => {
    throw new Error('ctx.host is not available yet')
  }

  constructor(init: HooksInit) {
    this.#objects = connectRun(init)
    this.#contexts.set(init.context, this.#objects.context)
    this.#meta = frozenCopy(init.meta)
    this.uncaught = this.#objects.pipe.uncaught
    this.uncaught.catch((error: unknown) => {
      this.#broken = { error }
      for (const invocation of this.#live.values())
        invocation.settle(failure(brokenBy(error)))
    })
  }

  on(event: unknown, second: unknown, third: unknown): void {
    if (!this.#loading)
      throw new TypeError('neo.on works only while the hook file loads')
    if (!isHookEvent(event))
      throw new TypeError(
        `neo.on: unknown hook event ${describe(event)}; expected one of ${HOOK_EVENTS.join(', ')}`,
      )
    const [filter, handler] =
      typeof second === 'function' && third === undefined
        ? [undefined, second]
        : [second, third]
    if (typeof handler !== 'function')
      throw new TypeError(`neo.on('${event}'): the handler must be a function`)
    this.#registrations.push(
      Object.freeze({
        event,
        ...readFilter(event, filter),
        handler: handler as HookHandler,
        index: this.#registrations.length + 1,
      }),
    )
  }

  finishLoading(): string {
    this.#loading = false
    return stringify(
      this.#registrations.map(({ event, kinds, groups }) => ({
        event,
        kinds,
        groups,
      })),
    )
  }

  invoke(
    id: number,
    event: string,
    payload: string,
    ship: Ship | undefined,
    marker: string | undefined,
  ): Promise<string> {
    // Registration is fixed by the first invocation even if Rust never called `finishLoading`,
    // so a handler's own `neo.on` always throws.
    this.#loading = false
    return new Promise<string>((resolve) => {
      if (this.#broken) {
        resolve(failure(brokenBy(this.#broken.error)))
        return
      }
      let invocation: Invocation
      try {
        invocation = { controller: new AbortController(), settle: resolve }
      } catch (error) {
        resolve(failure(`${event}: ${describe(error)}`))
        return
      }
      this.#live.set(id, invocation)
      this.#decide(event, payload, invocation.controller.signal, ship, marker)
        .then(resolve, (error: unknown) =>
          resolve(failure(`${event}: ${describe(error)}`)),
        )
        .finally(() => {
          if (this.#live.get(id) === invocation) this.#live.delete(id)
        })
    })
  }

  abort(id: number, reason: string): void {
    const invocation = this.#live.get(id)
    if (!invocation) return
    const error = new Error(String(reason))
    error.name = 'AbortError'
    invocation.controller.abort(error)
  }

  /** The decision JSON for one invocation. Throws only for what no handler caused (a payload
   * it cannot read, a guid Rust does not know), which `invoke` turns into `fail`. */
  async #decide(
    event: string,
    payloadText: string,
    signal: AbortSignal,
    ship: Ship | undefined,
    marker: string | undefined,
  ): Promise<string> {
    if (!isHookEvent(event))
      return failure(`unknown hook event ${describe(event)}`)
    const payload = readPayload(payloadText)
    const action = event === 'beforeAction' || event === 'afterAction'
    const kind = action ? nullableText(payload, 'kind') : null
    const group = action ? nullableText(payload, 'group') : null
    const handlers = this.#registrations.filter(
      (registration) =>
        registration.event === event && wants(registration, kind, group),
    )
    const objects = this.#objectsFor(action, payload)
    const ctx: HookContext = Object.freeze({
      meta: this.#meta,
      state: this.#state,
      signal,
      trace: this.#trace,
      host: this.#host,
    })
    const exporter =
      typeof ship === 'function' && typeof marker === 'string'
        ? (value: unknown) => exportValue(value, ship, marker)
        : (value: unknown) => stringify(value)
    if (event === 'beforeRun' || event === 'beforeAction')
      return await this.#before(event, handlers, payloadText, objects, ctx)
    return await this.#after(
      event,
      handlers,
      payloadText,
      payload,
      objects,
      ctx,
      exporter,
    )
  }

  /**
   * beforeRun and beforeAction: the first `block` wins and later handlers never run;
   * `replaceArgs` chains, each later handler seeing the replaced arguments.
   */
  async #before(
    event: 'beforeRun' | 'beforeAction',
    handlers: Registration[],
    payloadText: string,
    objects: EventObjects,
    ctx: HookContext,
  ): Promise<string> {
    let args: string | undefined
    for (const registration of handlers) {
      const stopped = this.#stopped(ctx.signal, registration)
      if (stopped) return failure(stopped)
      const data: Payload = parse(payloadText)
      if (args !== undefined) data.args = parse(args)
      const view = this.#view(event, data, objects)
      const checked = await run(registration, view, ctx, (returned) =>
        checkBefore(returned, event === 'beforeAction'),
      )
      if (typeof checked === 'string') return failure(checked)
      if ('block' in checked)
        return stringify({ decision: 'block', reason: checked.block })
      if ('replaceArgs' in checked) args = checked.replaceArgs
    }
    return args === undefined
      ? stringify({ decision: 'allow' })
      : `{"decision":"replaceArgs","args":${args}}`
  }

  /**
   * afterRun and afterAction: each handler sees the result as earlier ones replaced it, and
   * notes accumulate. A replacement is exported once, after the last handler.
   */
  async #after(
    event: 'afterRun' | 'afterAction',
    handlers: Registration[],
    payloadText: string,
    payload: Payload,
    objects: EventObjects,
    ctx: HookContext,
    exporter: (value: unknown) => string | undefined,
  ): Promise<string> {
    let replaced: { value: unknown } | undefined
    const notes: string[] = []
    for (const registration of handlers) {
      const stopped = this.#stopped(ctx.signal, registration)
      if (stopped) return failure(stopped)
      const view = this.#view(event, parse(payloadText), objects, replaced)
      const checked = await run(registration, view, ctx, checkAfter)
      if (typeof checked === 'string') return failure(checked)
      if (checked.replace) {
        // Contract §5: only a script that returned has a result to replace; replacing an
        // error, a timeout or a cancellation would hide how the run ended.
        if (event === 'afterRun' && payload.status !== 'ok')
          return failure(
            `${label(registration)} replaced the result of a run that ended ${describe(payload.status)}`,
          )
        replaced = checked.replace
      }
      if (checked.note !== undefined) notes.push(checked.note)
    }
    if (!replaced) return stringify({ decision: 'keep', notes })
    let value: string | undefined
    try {
      value = exporter(replaced.value)
    } catch (error) {
      return failure(
        `${event}: the replacement cannot be exported: ${describe(error)}`,
      )
    }
    const field = value === undefined ? '' : `"value":${value},`
    return `{"decision":"replace",${field}"notes":${stringify(notes)}}`
  }

  /** Why no further handler may start, or `undefined` while one may. */
  #stopped(signal: AbortSignal, next: Registration): string | undefined {
    if (this.#broken) return brokenBy(this.#broken.error)
    if (signal.aborted)
      return `aborted before ${label(next)}: ${describe(signal.reason)}`
    return undefined
  }

  /** The `event` one handler receives: a fresh copy of the payload's data, so a handler that
   * edits its `event` changes nothing another handler sees, and the shared real objects. */
  #view(
    event: HookEvent,
    data: Payload,
    objects: EventObjects,
    replaced?: { value: unknown },
  ): Record<string, unknown> {
    const view: Record<string, unknown> = {}
    if (event === 'beforeRun') view.script = data.script
    else if (event === 'afterRun') {
      view.status = data.status
      if (replaced) view.value = replaced.value
      else if (Object.hasOwn(data, 'value')) view.value = data.value
      if (Object.hasOwn(data, 'error')) view.error = data.error
    } else {
      view.kind = data.kind ?? null
      view.group = data.group ?? null
      view.method = data.method
      view.args = data.args
      if (Object.hasOwn(data, 'location')) view.location = data.location
      if (event === 'afterAction') {
        if (replaced) view.result = replaced.value
        else if (Object.hasOwn(data, 'result')) view.result = data.result
        if (Object.hasOwn(data, 'error')) view.error = data.error
      }
    }
    if (objects.page) view.page = objects.page
    if (objects.frame) view.frame = objects.frame
    view.context = objects.context
    if (objects.locator) view.locator = objects.locator
    if (objects.element) view.element = objects.element
    view.meta = this.#meta
    return view
  }

  /**
   * The payload's guids as this realm's objects. The run page is the one `createHooks` built;
   * any other page is the object this realm already holds for its guid, else one built from its
   * `initializer` getter, as `context.pages()` builds a page. A frame belongs to the page its
   * guid names (`frame@T/F` is `page@T`'s).
   *
   * An element action's `element` (F4) is the handle Rust pinned in this realm's view of the
   * run's handles, `{guid, preview}`: an `ElementHandle` built from it, as from any handle record
   * Rust creates, its timeouts the call's frame's page's. The agent's realm never sees its guid.
   */
  #objectsFor(action: boolean, payload: Payload): EventObjects {
    const context = this.#contextFor(text(payload, 'context'))
    if (!action)
      return { context, page: this.#pageFor(text(payload, 'page'), context) }
    const objects: EventObjects = { context }
    if (payload.page !== undefined)
      objects.page = this.#pageFor(text(payload, 'page'), context)
    if (payload.frame !== undefined)
      objects.frame = this.#frameFor(text(payload, 'frame'), context)
    if (payload.locator !== undefined) {
      const locator = record(payload, 'locator')
      objects.locator = this.#frameFor(text(locator, 'frame'), context).locator(
        text(locator, 'selector'),
      )
    }
    if (payload.element !== undefined) {
      const element = record(payload, 'element')
      objects.element = new ElementHandle(
        this.#objects.connection,
        text(element, 'guid'),
        { preview: text(element, 'preview') },
        objects.frame,
      )
    }
    return objects
  }

  /** The run's context for its guid; another guid (a context the agent created) gets a context
   * of its own, registered with the browser so its pages are found by guid like the run's. */
  #contextFor(guid: string): BrowserContext {
    let context = this.#contexts.get(guid)
    if (!context) {
      context = new BrowserContext(
        guid,
        this.#objects.connection,
        this.#objects.pipe,
      )
      this.#objects.browser._didCreateContext(context)
      this.#contexts.set(guid, context)
    }
    return context
  }

  /** The page for `guid`: the object this realm holds for it, else one built in `context`. */
  #pageFor(guid: string, context: BrowserContext): Page {
    return (
      this.#objects.browser._knownPage(guid) ??
      context._pageFrom(get(guid, 'initializer'))
    )
  }

  #frameFor(guid: string, context: BrowserContext): Frame {
    return this.#pageFor(pageGuidOf(guid), context)._frameForGuid(guid)
  }
}

/** Runs one handler and checks what it returned: the checked decision, or how it failed. */
async function run<T>(
  registration: Registration,
  view: Record<string, unknown>,
  ctx: HookContext,
  check: (returned: unknown) => T,
): Promise<T | string> {
  const { handler } = registration
  let returned: unknown
  try {
    returned = await handler(view, ctx)
  } catch (error) {
    return `${label(registration)} threw ${describe(error)}`
  }
  try {
    return check(returned)
  } catch (error) {
    return `${label(registration)} returned an invalid decision: ${describe(error)}`
  }
}

/**
 * A before-event return: `undefined` or `{ allow: true }` allows, `{ block: <non-empty
 * string> }` blocks, and on beforeAction `{ replaceArgs: <object> }` replaces the arguments,
 * as JSON text. Anything else throws, which is a failure.
 */
function checkBefore(returned: unknown, action: boolean): Before {
  if (returned === undefined) return { allow: true }
  const [key, ...others] = ownKeys(returned)
  if (key === undefined || others.length)
    throw new TypeError('expected exactly one of allow, block, replaceArgs')
  const value = (returned as Record<string, unknown>)[key as string]
  if (key === 'allow') {
    if (value !== true) throw new TypeError('allow must be true')
    return { allow: true }
  }
  if (key === 'block') {
    if (typeof value !== 'string' || value === '')
      throw new TypeError('block must be a non-empty string')
    return { block: value }
  }
  if (key === 'replaceArgs' && action) {
    if (typeof value !== 'object' || value === null || Array.isArray(value))
      throw new TypeError('replaceArgs must be an object')
    const args = stringify(value)
    if (typeof args !== 'string' || !args.startsWith('{'))
      throw new TypeError('replaceArgs must be a JSON object')
    return { replaceArgs: args }
  }
  throw new TypeError(`unexpected key ${describe(String(key))}`)
}

/** An after-event return: `undefined` keeps; `{ replace }`, `{ note: <string> }` or both.
 * `replace` counts when it is an own key, so `{ replace: undefined }` replaces with
 * `undefined`. Anything else throws, which is a failure. */
function checkAfter(returned: unknown): After {
  if (returned === undefined) return {}
  const keys = ownKeys(returned)
  if (!keys.length) throw new TypeError('expected replace, note or both')
  for (const key of keys)
    if (key !== 'replace' && key !== 'note')
      throw new TypeError(`unexpected key ${describe(String(key))}`)
  const decision = returned as { replace?: unknown; note?: unknown }
  const after: After = {}
  if (Object.hasOwn(decision, 'replace'))
    after.replace = { value: decision.replace }
  if (Object.hasOwn(decision, 'note')) {
    if (typeof decision.note !== 'string')
      throw new TypeError('note must be a string')
    after.note = decision.note
  }
  return after
}

/** A returned decision's own keys, symbols and non-enumerable ones included, or a throw when
 * it is not an object. */
function ownKeys(returned: unknown): (string | symbol)[] {
  if (
    typeof returned !== 'object' ||
    returned === null ||
    Array.isArray(returned)
  )
    throw new TypeError(
      `expected undefined or an object, got ${describe(returned)}`,
    )
  return Reflect.ownKeys(returned)
}

/** A `neo.on` filter as its registration keeps it: copies, frozen. */
function readFilter(event: HookEvent, filter: unknown): HookFilter {
  if (filter === undefined || filter === null) return {}
  if (event !== 'beforeAction' && event !== 'afterAction')
    throw new TypeError(`neo.on('${event}'): only action events take a filter`)
  if (typeof filter !== 'object' || Array.isArray(filter))
    throw new TypeError(
      `neo.on('${event}'): a filter is an object { kinds?, groups? }`,
    )
  for (const key of Reflect.ownKeys(filter))
    if (key !== 'kinds' && key !== 'groups')
      throw new TypeError(
        `neo.on('${event}'): unknown filter key ${describe(String(key))}`,
      )
  const { kinds, groups } = filter as Record<string, unknown>
  const read = {
    kinds: filterNames(event, 'kinds', kinds),
    groups: filterNames(event, 'groups', groups),
  }
  if (!read.kinds && !read.groups)
    throw new TypeError(
      `neo.on('${event}'): a filter names kinds, groups or both`,
    )
  return read
}

function filterNames(
  event: HookEvent,
  key: string,
  names: unknown,
): readonly string[] | undefined {
  if (names === undefined) return undefined
  if (!Array.isArray(names) || !names.every((name) => typeof name === 'string'))
    throw new TypeError(
      `neo.on('${event}'): filter ${key} must be an array of strings`,
    )
  return Object.freeze([...names])
}

/** Whether `registration` wants an action of `kind` and `group`, as Rust's `wants` decides: no
 * filter wants every action; a `null` kind or group (until F1 classifies actions) matches none. */
function wants(
  registration: Registration,
  kind: string | null,
  group: string | null,
): boolean {
  const { kinds, groups } = registration
  if (!kinds && !groups) return true
  return (
    (kind !== null && !!kinds?.includes(kind)) ||
    (group !== null && !!groups?.includes(group))
  )
}

function isHookEvent(event: unknown): event is HookEvent {
  return HOOK_EVENTS.includes(event as HookEvent)
}

function readPayload(payloadText: string): Payload {
  const payload: unknown = parse(payloadText)
  if (typeof payload !== 'object' || payload === null || Array.isArray(payload))
    throw new TypeError('the payload is not a JSON object')
  return payload as Payload
}

/** `object[key]` as a string, or a `TypeError` naming it. */
function text(object: Payload, key: string): string {
  const value = object[key]
  if (typeof value !== 'string')
    throw new TypeError(`payload ${key}: expected a string`)
  return value
}

/** `object[key]` as a string, `null` when absent or null, or a `TypeError` naming it. */
function nullableText(object: Payload, key: string): string | null {
  const value = object[key]
  if (value === undefined || value === null) return null
  if (typeof value !== 'string')
    throw new TypeError(`payload ${key}: expected a string or null`)
  return value
}

/** `object[key]` as a JSON object, or a `TypeError` naming it. */
function record(object: Payload, key: string): Payload {
  const value = object[key]
  if (typeof value !== 'object' || value === null || Array.isArray(value))
    throw new TypeError(`payload ${key}: expected an object`)
  return value as Payload
}

/** A deep-frozen JSON copy of the run's meta, `null` when the host sent none. */
function frozenCopy(meta: unknown): unknown {
  const json = meta === undefined ? undefined : stringify(meta)
  return json === undefined ? null : deepFreeze(parse(json))
}

function deepFreeze(value: unknown): unknown {
  if (typeof value === 'object' && value !== null) {
    for (const child of Object.values(value)) deepFreeze(child)
    Object.freeze(value)
  }
  return value
}

function failure(error: string): string {
  return stringify({ decision: 'fail', error })
}

function brokenBy(error: unknown): string {
  return `a listener error went unhandled in the hook realm: ${describe(error)}`
}

/** How the hook trace names a handler: its event and its place in the hook file. */
function label(registration: Registration): string {
  return `${registration.event} handler #${registration.index}`
}

/**
 * One line for the hook trace about any value, which never throws: an error's name and message,
 * a string as itself, anything else as JSON, or as `String` where JSON has nothing.
 */
function describe(value: unknown): string {
  try {
    if (value instanceof Error) return `${value.name}: ${value.message}`
    if (typeof value === 'string') return value
    return stringify(value) ?? String(value)
  } catch {
    try {
      return String(value)
    } catch {
      return 'a value that cannot be printed'
    }
  }
}
