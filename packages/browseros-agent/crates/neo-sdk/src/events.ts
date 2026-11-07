/**
 * Playwright: `client/eventEmitter.ts` `EventEmitter`
 * Playwright: `client/channelOwner.ts` `ChannelOwner`
 *
 * The event half of Playwright's client inside a run's realm (neo-runtime `events.rs` is the
 * other half). `EventEmitter` is Playwright's port of Node's emitter, kept line for line;
 * `ChannelOwner` keeps its listener-count subscriptions; `EventPipe` is how Neo's events reach
 * them, in the order Playwright's connection dispatches its server's messages:
 *
 * ```text
 *  page.on('console', f) ─ 0 → 1 listeners ─▶ __neo_runtime.subscribe({guid, event, enabled})
 *  a call's reply {reply, events: W} ─▶ deliver(W): drain(W), dispatch by guid ─▶ the caller
 *  idle: nextEvents() resolves ─▶ deliver(all) ─▶ re-armed while anything is subscribed
 *  BrowserContext._onEvent('console', params) ─▶ page.emit('console', …), context.emit(…)
 * ```
 *
 * Listener semantics are Node's, as in Playwright: a listener that throws throws out of `emit`,
 * an async listener's rejection goes to an `error` listener or is uncaught. Uncaught means Node
 * exits; Neo ends the run with that error (`EventPipe.uncaught`, raced in neo-runtime's realm).
 */

import { drain, nextEvents, subscribe } from './runtime-bridge'

// biome-ignore lint/suspicious/noExplicitAny: Node's EventEmitter listener type.
type Listener = (...args: any[]) => any
type EventType = string | symbol
type EventMap = Record<EventType, Listener | Listener[]>

/**
 * Platform: Node's `events.defaultMaxListeners` is 10 (nodejs.org/api/events.html), the value
 * Playwright's `getMaxListeners` falls back to through `NodeEventEmitter.defaultMaxListeners`;
 * QuickJS has no `events` module.
 */
const DEFAULT_MAX_LISTENERS = 10

/** Neo: where a listener error nobody handles goes; the run's `EventPipe` sets it. */
let uncaughtListenerError: ((error: unknown) => void) | undefined

/** Neo: routes listener errors nobody handles to `report` (or back to rethrowing). */
export function onUncaughtListenerError(
  report: ((error: unknown) => void) | undefined,
): void {
  uncaughtListenerError = report
}

/** Neo: a channel handler's error (a route handler's), which Playwright's channel emitter
 * rethrows as an unhandled rejection: the run ends with it. */
export function reportUncaught(error: unknown): void {
  if (uncaughtListenerError) uncaughtListenerError(error)
  else throw error
}

/** Playwright: `client/eventEmitter.ts` `EventEmitter` */
export class EventEmitter {
  private _events: EventMap | undefined = undefined
  private _eventsCount = 0
  private _maxListeners: number | undefined = undefined
  readonly _pendingHandlers = new Map<EventType, Set<Promise<void>>>()
  private _rejectionHandler: ((error: Error) => void) | undefined

  constructor() {
    if (
      this._events === undefined ||
      this._events === Object.getPrototypeOf(this)._events
    ) {
      this._events = Object.create(null)
      this._eventsCount = 0
    }
    this._maxListeners = this._maxListeners || undefined
    this.on = this.addListener
    this.off = this.removeListener
  }

  setMaxListeners(n: number): this {
    if (typeof n !== 'number' || n < 0 || Number.isNaN(n))
      throw new RangeError(
        `The value of "n" is out of range. It must be a non-negative number. Received ${n}.`,
      )
    this._maxListeners = n
    return this
  }

  getMaxListeners(): number {
    return this._maxListeners === undefined
      ? DEFAULT_MAX_LISTENERS
      : this._maxListeners
  }

  // biome-ignore lint/suspicious/noExplicitAny: Node's EventEmitter signature.
  emit(type: EventType, ...args: any[]): boolean {
    const events = this._events
    if (events === undefined) return false

    const handler = events?.[type]
    if (handler === undefined) return false

    if (typeof handler === 'function') {
      this._callHandler(type, handler, args)
    } else {
      const len = handler.length
      const listeners = handler.slice()
      for (let i = 0; i < len; ++i) this._callHandler(type, listeners[i], args)
    }
    return true
  }

  // biome-ignore lint/suspicious/noExplicitAny: Node's EventEmitter signature.
  private _callHandler(type: EventType, handler: Listener, args: any[]): void {
    let promise: Promise<void>
    try {
      const result = Reflect.apply(handler, this, args)
      const then = result?.then
      if (typeof then !== 'function') return
      promise = new Promise<void>((resolve, reject) =>
        then.call(result, resolve, reject),
      )
    } catch (error) {
      this._handleListenerError(type, error)
      return
    }
    let set = this._pendingHandlers.get(type)
    if (!set) {
      set = new Set()
      this._pendingHandlers.set(type, set)
    }
    set.add(promise)
    promise
      .catch((e) => {
        if (this._rejectionHandler) this._rejectionHandler(e)
        else this._handleListenerError(type, e)
      })
      // Deviation: `client/eventEmitter.ts` `EventEmitter._callHandler`: `this._handleListenerError(type, e);`
      // There the rethrow is an unhandled rejection, and Node exits; QuickJS ignores one, so
      // Neo hands it to the run, which ends with it.
      .catch((error) => {
        if (uncaughtListenerError) uncaughtListenerError(error)
        else throw error
      })
      .finally(() => set.delete(promise))
  }

  private _handleListenerError(type: EventType, error: unknown): void {
    if (type === 'error' || !this.listenerCount('error')) throw error
    this.emit('error', error)
  }

  addListener(type: EventType, listener: Listener): this {
    return this._addListener(type, listener, false)
  }

  on(type: EventType, listener: Listener): this {
    return this._addListener(type, listener, false)
  }

  private _addListener(
    type: EventType,
    listener: Listener,
    prepend: boolean,
  ): this {
    checkListener(listener)
    let events = this._events
    let existing: Listener | Listener[] | undefined
    if (events === undefined) {
      events = this._events = Object.create(null)
      this._eventsCount = 0
    } else {
      // To avoid recursion in the case that type === "newListener"! Before
      // adding it to the listeners, first emit "newListener".
      if (events.newListener !== undefined) {
        this.emit('newListener', type, unwrapListener(listener))

        // Re-assign `events` because a newListener handler could have caused the
        // this._events to be assigned to a new object
        events = this._events as EventMap
      }
      existing = events[type]
    }

    if (existing === undefined) {
      // Optimize the case of one listener. Don't need the extra array object.
      existing = (events as EventMap)[type] = listener
      ++this._eventsCount
    } else {
      if (typeof existing === 'function') {
        // Adding the second element, need to change to array.
        existing = (events as EventMap)[type] = prepend
          ? [listener, existing]
          : [existing, listener]
        // If we've already got an array, just append.
      } else if (prepend) {
        existing.unshift(listener)
      } else {
        existing.push(listener)
      }

      // Check for listener leak
      const m = this.getMaxListeners()
      // biome-ignore lint/suspicious/noExplicitAny: Node marks the array itself.
      const marked = existing as any
      if (m > 0 && existing.length > m && !marked.warned) {
        marked.warned = true
        // No error code for this since it is a Warning
        const w = new Error(
          'Possible EventEmitter memory leak detected. ' +
            existing.length +
            ' ' +
            String(type) +
            ' listeners ' +
            'added. Use emitter.setMaxListeners() to ' +
            'increase limit',
        ) as Error & { emitter?: unknown; type?: EventType; count?: number }
        w.name = 'MaxListenersExceededWarning'
        w.emitter = this
        w.type = type
        w.count = existing.length
        // Deviation: `client/eventEmitter.ts` `EventEmitter._addListener`: `if (!isUnderTest()) {`
        // Neo runs are never Playwright's own tests, so the warning always goes to the run log.
        console.warn(w)
      }
    }

    return this
  }

  prependListener(type: EventType, listener: Listener): this {
    return this._addListener(type, listener, true)
  }

  once(type: EventType, listener: Listener): this {
    checkListener(listener)
    this.on(type, new OnceWrapper(this, type, listener).wrapperFunction)
    return this
  }

  prependOnceListener(type: EventType, listener: Listener): this {
    checkListener(listener)
    this.prependListener(
      type,
      new OnceWrapper(this, type, listener).wrapperFunction,
    )
    return this
  }

  removeListener(type: EventType, listener: Listener): this {
    checkListener(listener)

    const events = this._events
    if (events === undefined) return this

    const list = events[type]
    if (list === undefined) return this

    // biome-ignore lint/suspicious/noExplicitAny: a once wrapper carries its listener.
    if (list === listener || (list as any).listener === listener) {
      if (--this._eventsCount === 0) {
        this._events = Object.create(null)
      } else {
        delete events[type]
        if (events.removeListener)
          // biome-ignore lint/suspicious/noExplicitAny: a once wrapper carries its listener.
          this.emit('removeListener', type, (list as any).listener ?? listener)
      }
    } else if (typeof list !== 'function') {
      let position = -1
      let originalListener: Listener | undefined

      for (let i = list.length - 1; i >= 0; i--) {
        if (list[i] === listener || wrappedListener(list[i]) === listener) {
          originalListener = wrappedListener(list[i])
          position = i
          break
        }
      }

      if (position < 0) return this

      if (position === 0) list.shift()
      else list.splice(position, 1)

      if (list.length === 1) events[type] = list[0]

      if (events.removeListener !== undefined)
        this.emit('removeListener', type, originalListener || listener)
    }

    return this
  }

  off(type: EventType, listener: Listener): this {
    return this.removeListener(type, listener)
  }

  removeAllListeners(type?: EventType): this
  removeAllListeners(
    type: EventType | undefined,
    options: { behavior?: 'wait' | 'ignoreErrors' | 'default' },
  ): Promise<void>
  removeAllListeners(
    type?: EventType,
    options?: { behavior?: 'wait' | 'ignoreErrors' | 'default' },
  ): this | Promise<void> {
    this._removeAllListeners(type)
    if (!options) return this

    if (options.behavior === 'wait') {
      const errors: Error[] = []
      this._rejectionHandler = (error) => errors.push(error)
      return this._waitFor(type).then(() => {
        if (errors.length) throw errors[0]
      })
    }

    if (options.behavior === 'ignoreErrors') this._rejectionHandler = () => {}

    return Promise.resolve()
  }

  private _removeAllListeners(type?: EventType) {
    const events = this._events
    if (!events) return

    // not listening for removeListener, no need to emit
    if (!events.removeListener) {
      if (type === undefined) {
        this._events = Object.create(null)
        this._eventsCount = 0
      } else if (events[type] !== undefined) {
        if (--this._eventsCount === 0) this._events = Object.create(null)
        else delete events[type]
      }
      return
    }

    // emit removeListener for all listeners on all events
    if (type === undefined) {
      const keys = Object.keys(events)
      let key: string
      for (let i = 0; i < keys.length; ++i) {
        key = keys[i]
        if (key === 'removeListener') continue
        this._removeAllListeners(key)
      }
      this._removeAllListeners('removeListener')
      this._events = Object.create(null)
      this._eventsCount = 0
      return
    }

    const listeners = events[type]

    if (typeof listeners === 'function') {
      this.removeListener(type, listeners)
    } else if (listeners !== undefined) {
      // LIFO order
      for (let i = listeners.length - 1; i >= 0; i--)
        this.removeListener(type, listeners[i])
    }
  }

  listeners(type: EventType): Listener[] {
    return this._listeners(this, type, true)
  }

  rawListeners(type: EventType): Listener[] {
    return this._listeners(this, type, false)
  }

  listenerCount(type: EventType): number {
    const events = this._events
    if (events !== undefined) {
      const listener = events[type]
      if (typeof listener === 'function') return 1
      if (listener !== undefined) return listener.length
    }
    return 0
  }

  eventNames(): Array<string | symbol> {
    return this._eventsCount > 0 && this._events
      ? Reflect.ownKeys(this._events)
      : []
  }

  private async _waitFor(type?: EventType) {
    let promises: Promise<void>[] = []
    if (type) {
      promises = [...(this._pendingHandlers.get(type) || [])]
    } else {
      promises = []
      for (const [, pending] of this._pendingHandlers) promises.push(...pending)
    }
    await Promise.all(promises)
  }

  private _listeners(
    target: EventEmitter,
    type: EventType,
    unwrap: boolean,
  ): Listener[] {
    const events = target._events

    if (events === undefined) return []

    const listener = events[type]
    if (listener === undefined) return []

    if (typeof listener === 'function')
      return unwrap ? [unwrapListener(listener)] : [listener]

    return unwrap ? unwrapListeners(listener) : listener.slice()
  }
}

/** Playwright: `client/eventEmitter.ts` `checkListener` */
// biome-ignore lint/suspicious/noExplicitAny: Node checks any value.
function checkListener(listener: any) {
  if (typeof listener !== 'function')
    throw new TypeError(
      `The "listener" argument must be of type Function. Received type ${typeof listener}`,
    )
}

/** Playwright: `client/eventEmitter.ts` `OnceWrapper` */
class OnceWrapper {
  private _fired = false
  readonly wrapperFunction: (...args: unknown[]) => Promise<void> | void
  readonly _listener: Listener
  private _eventEmitter: EventEmitter
  private _eventType: EventType

  constructor(
    eventEmitter: EventEmitter,
    eventType: EventType,
    listener: Listener,
  ) {
    this._eventEmitter = eventEmitter
    this._eventType = eventType
    this._listener = listener
    this.wrapperFunction = this._handle.bind(this)
    // biome-ignore lint/suspicious/noExplicitAny: Node marks the wrapper with its listener.
    ;(this.wrapperFunction as any).listener = listener
  }

  private _handle(...args: unknown[]) {
    if (this._fired) return
    this._fired = true
    this._eventEmitter.removeListener(this._eventType, this.wrapperFunction)
    return this._listener.apply(this._eventEmitter, args)
  }
}

/** Playwright: `client/eventEmitter.ts` `unwrapListener` */
function unwrapListener(l: Listener): Listener {
  return wrappedListener(l) ?? l
}

/** Playwright: `client/eventEmitter.ts` `unwrapListeners` */
function unwrapListeners(arr: Listener[]): Listener[] {
  return arr.map((l) => wrappedListener(l) ?? l)
}

/** Playwright: `client/eventEmitter.ts` `wrappedListener` */
function wrappedListener(l: Listener): Listener {
  // biome-ignore lint/suspicious/noExplicitAny: a once wrapper carries its listener.
  return (l as any).listener
}

/**
 * The listener events a receiver knows, by how Neo treats each: `served` ones are produced and
 * subscribed to on their first listener; `local` ones the SDK emits itself, with no subscription
 * (a context's `close`); `quiet` ones are Playwright's but never happen in a run, and take
 * listeners as Playwright's would; `unserved` ones are Playwright's and not produced by Neo yet,
 * so a listener for one throws rather than wait forever in silence. Any other name is an
 * ordinary emitter event, as in Playwright.
 */
export interface EventTable {
  receiver: string
  served: readonly string[]
  local?: readonly string[]
  quiet: readonly string[]
  unserved: readonly string[]
}

/**
 * Playwright: `client/channelOwner.ts` `ChannelOwner`
 *
 * Playwright's `ChannelOwner` subscriptions: the first listener of a served event turns its
 * subscription on, removing the last turns it off. Playwright gates eight events this way and
 * sends the rest regardless; Neo gates every event it produces, so a run that listens to
 * nothing makes Rust produce nothing.
 */
export abstract class ChannelOwner extends EventEmitter {
  readonly _guid: string
  readonly #pipe: EventPipe
  readonly #table: EventTable

  constructor(guid: string, pipe: EventPipe, table: EventTable) {
    super()
    this.setMaxListeners(0)
    this._guid = guid
    this.#pipe = pipe
    this.#table = table
    pipe._register(this)
  }

  /** One event Rust sent this object: Playwright's channel event `method` with `params`. */
  abstract _onEvent(method: string, params: unknown): void

  #served(event: EventType): event is string {
    return typeof event === 'string' && this.#table.served.includes(event)
  }

  /** Neo: a listener for a Playwright event Neo does not produce yet would never fire. */
  _refuseUnserved(event: EventType): void {
    if (typeof event === 'string' && this.#table.unserved.includes(event))
      throw new Error(
        `Neo does not serve the "${event}" event on ${this.#table.receiver} yet`,
      )
  }

  /** Neo: a channel event the SDK handles itself, with no public listener (such as Page's
   * `locatorHandlerTriggered`), subscribed while the SDK needs it; `bindingCall` is turned on for
   * good once a binding is exposed, so its calls reach the realm while the script waits. */
  _subscribeChannelEvent(event: string, enabled: boolean): void {
    this.#pipe._subscribe(this._guid, event, enabled)
  }

  #updateSubscription(event: EventType, enabled: boolean): void {
    if (this.#served(event)) this.#pipe._subscribe(this._guid, event, enabled)
  }

  override on(event: EventType, listener: Listener): this {
    this._refuseUnserved(event)
    if (!this.listenerCount(event)) this.#updateSubscription(event, true)
    super.on(event, listener)
    return this
  }

  override addListener(event: EventType, listener: Listener): this {
    this._refuseUnserved(event)
    if (!this.listenerCount(event)) this.#updateSubscription(event, true)
    super.addListener(event, listener)
    return this
  }

  override prependListener(event: EventType, listener: Listener): this {
    this._refuseUnserved(event)
    if (!this.listenerCount(event)) this.#updateSubscription(event, true)
    super.prependListener(event, listener)
    return this
  }

  override off(event: EventType, listener: Listener): this {
    super.off(event, listener)
    if (!this.listenerCount(event)) this.#updateSubscription(event, false)
    return this
  }

  override removeListener(event: EventType, listener: Listener): this {
    super.removeListener(event, listener)
    if (!this.listenerCount(event)) this.#updateSubscription(event, false)
    return this
  }

  override removeAllListeners(type?: EventType): this
  override removeAllListeners(
    type: EventType | undefined,
    options: { behavior?: 'wait' | 'ignoreErrors' | 'default' },
  ): Promise<void>
  override removeAllListeners(
    type?: EventType,
    options?: { behavior?: 'wait' | 'ignoreErrors' | 'default' },
  ): this | Promise<void> {
    const listening = this.#table.served.filter(
      (event) => this.listenerCount(event) > 0,
    )
    const result = options
      ? super.removeAllListeners(type, options)
      : super.removeAllListeners(type)
    // Deviation: `client/channelOwner.ts` `ChannelOwner.removeListener`: `if (!this.listenerCount(event))`
    // Playwright's removeAllListeners leaves its server subscriptions on; Neo turns off each one
    // left without listeners, so Rust stops producing what nobody hears.
    for (const event of listening)
      if (!this.listenerCount(event)) this.#updateSubscription(event, false)
    return result
  }
}

/** One event as Rust queues it (neo-runtime `events.rs`). */
export interface RawEvent {
  seq: number
  guid: string
  method: string
  params: unknown
}

/**
 * Neo: the run's event pipe. `deliver` is the only consumer of Rust's queue; it runs in the first
 * reaction after a native promise resolves, so it never interleaves with itself, and every
 * listener it calls runs before the reply's caller continues. While anything is subscribed, one
 * `nextEvents()` wake stays pending, so events also reach listeners while the script waits on a
 * timer or `waitForEvent`.
 */
export class EventPipe {
  readonly #targets = new Map<string, ChannelOwner>()
  readonly #subscriptions = new Set<string>()
  #waking = false
  #failed = false
  #disconnected = false
  #reject: (error: unknown) => void = () => {}
  /** Rejects with the first listener error nobody handled; never resolves. */
  readonly uncaught: Promise<never>

  constructor() {
    this.uncaught = new Promise<never>((_, reject) => {
      this.#reject = reject
    })
    // Handled by neo-runtime's race; this keeps QuickJS from tracking it as unhandled meanwhile.
    this.uncaught.catch(() => {})
  }

  _register(target: ChannelOwner): void {
    this.#targets.set(target._guid, target)
  }

  /** Neo: the run's browser closed (`Browser.close`): Rust stops producing, and nothing queued
   * or later is delivered, as Playwright's closed connection delivers nothing. */
  _disconnect(): void {
    for (const key of this.#subscriptions) {
      const [guid = '', event = ''] = key.split('\n')
      subscribe(guid, event, false)
    }
    this.#subscriptions.clear()
    this.#disconnected = true
  }

  _subscribe(guid: string, event: string, enabled: boolean): void {
    if (this.#disconnected) return
    subscribe(guid, event, enabled)
    const key = `${guid}\n${event}`
    if (enabled) this.#subscriptions.add(key)
    else this.#subscriptions.delete(key)
    if (this.#subscriptions.size) this.#arm()
  }

  /** Runs the listeners of every queued event up to seq `until` (all of them for -1). */
  deliver(until: number): void {
    const events = drain(until)
    if (this.#disconnected) return
    for (const event of events) {
      const target = this.#targets.get(event.guid)
      if (!target) continue
      try {
        target._onEvent(event.method, event.params)
      } catch (error) {
        this.report(error)
      }
    }
  }

  /** Ends the run with `error`, the first listener error nobody handled. */
  report(error: unknown): void {
    if (this.#failed) return
    this.#failed = true
    this.#reject(error)
  }

  #arm(): void {
    if (this.#waking) return
    this.#waking = true
    nextEvents().then(() => {
      this.#waking = false
      this.deliver(-1)
      if (this.#subscriptions.size) this.#arm()
    })
  }
}
