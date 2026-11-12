/**
 * The Node-style emitter playwright-core's classes extend, so `page.on`, `once`, `off` and the
 * rest behave as scripts expect. Subclasses learn when an event gains its first listener or
 * loses its last one (`_listenersChanged`), which is how Page and BrowserContext tell Rust which
 * events this run wants, and may refuse an event Neo never reports (`_checkEvent`).
 *
 * Like playwright-core's client emitter, it keeps the promise an async listener returns until
 * it settles, so `removeAllListeners(event, { behavior })` can wait for the listeners still
 * running (`wait`) or silence their later failures (`ignoreErrors`).
 */
export type Listener = (...args: never[]) => unknown

type Registered = { listener: Listener; once: boolean }

/** `removeAllListeners`' options, as upstream's `page.removeAllListeners(type, options)`. */
export type RemoveAllListenersOptions = {
  behavior?: 'wait' | 'ignoreErrors' | 'default'
}

export class EventEmitter {
  readonly #registered = new Map<string | symbol, Registered[]>()
  #maxListeners = 10
  /** Each event's async listener calls that have not settled yet. */
  readonly #pending = new Map<string | symbol, Set<Promise<unknown>>>()
  /**
   * Where an async listener's rejection goes once `removeAllListeners` chose a behavior;
   * without one it stays an unhandled rejection, as an uncaught listener error is in Node.
   * Kept for the emitter's life, as upstream keeps it.
   */
  #rejectionHandler: ((error: unknown) => void) | undefined

  /** Throws for an event that would never fire, before any listener is kept. */
  protected _checkEvent(_event: string | symbol): void {}

  /** Called after `event` gained its first listener or lost its last one. */
  protected _listenersChanged(
    _event: string | symbol,
    _listening: boolean,
  ): void {}

  addListener(event: string | symbol, listener: Listener): this {
    return this.#add(event, listener, false, false)
  }

  on(event: string | symbol, listener: Listener): this {
    return this.#add(event, listener, false, false)
  }

  once(event: string | symbol, listener: Listener): this {
    return this.#add(event, listener, true, false)
  }

  prependListener(event: string | symbol, listener: Listener): this {
    return this.#add(event, listener, false, true)
  }

  prependOnceListener(event: string | symbol, listener: Listener): this {
    return this.#add(event, listener, true, true)
  }

  removeListener(event: string | symbol, listener: Listener): this {
    const list = this.#registered.get(event)
    if (!list) return this
    const index = list.findLastIndex((entry) => entry.listener === listener)
    if (index === -1) return this
    list.splice(index, 1)
    if (list.length === 0) {
      this.#registered.delete(event)
      this._listenersChanged(event, false)
    }
    return this
  }

  off(event: string | symbol, listener: Listener): this {
    return this.removeListener(event, listener)
  }

  /**
   * Without options, Node's synchronous form. With them, upstream's: the listeners are gone
   * at once and the promise says what happens to calls of them still running. `wait` settles
   * when those calls of `event` (of every event, with none named) have, rejecting with an
   * error one of them threw; `ignoreErrors` swallows their failures; `default` leaves them as
   * they were.
   */
  removeAllListeners(event?: string | symbol): this
  removeAllListeners(
    event: string | symbol | undefined,
    options: RemoveAllListenersOptions,
  ): Promise<void>
  removeAllListeners(
    event?: string | symbol,
    options?: RemoveAllListenersOptions,
  ): this | Promise<void> {
    const events = event === undefined ? [...this.#registered.keys()] : [event]
    for (const name of events) {
      if (!this.#registered.delete(name)) continue
      this._listenersChanged(name, false)
    }
    if (!options) return this
    if (options.behavior === 'wait') {
      const errors: unknown[] = []
      this.#rejectionHandler = (error) => errors.push(error)
      return this.#settled(event).then(() => {
        if (errors.length > 0) throw errors[0]
      })
    }
    if (options.behavior === 'ignoreErrors') this.#rejectionHandler = () => {}
    return Promise.resolve()
  }

  emit(event: string | symbol, ...args: unknown[]): boolean {
    const list = this.#registered.get(event)
    if (!list || list.length === 0) return false
    for (const entry of [...list]) {
      if (entry.once) this.removeListener(event, entry.listener)
      // A throwing listener must not stop the others or the event pump that called emit; its
      // error surfaces as an unhandled rejection, as an uncaught listener error does in Node.
      let returned: unknown
      try {
        returned = Reflect.apply(entry.listener, this, args)
      } catch (error) {
        void Promise.reject(error)
        continue
      }
      if (returned instanceof Promise) this.#track(event, returned)
    }
    return true
  }

  /** Keeps an async listener's call until it settles, and routes its rejection. */
  #track(event: string | symbol, call: Promise<unknown>): void {
    let calls = this.#pending.get(event)
    if (!calls) {
      calls = new Set()
      this.#pending.set(event, calls)
    }
    const pending = calls
    pending.add(call)
    void call
      .catch((error: unknown) => {
        const handler = this.#rejectionHandler
        if (!handler) throw error
        handler(error)
      })
      .finally(() => {
        pending.delete(call)
        if (pending.size === 0 && this.#pending.get(event) === pending)
          this.#pending.delete(event)
      })
  }

  /**
   * Resolves once the calls of `event`'s listeners running now have, or rejects with the first
   * that fails, as upstream's `_waitFor`.
   */
  async #settled(event: string | symbol | undefined): Promise<void> {
    const calls =
      event === undefined
        ? [...this.#pending.values()].flatMap((pending) => [...pending])
        : [...(this.#pending.get(event) ?? [])]
    await Promise.all(calls)
  }

  listeners(event: string | symbol): Listener[] {
    return (this.#registered.get(event) ?? []).map((entry) => entry.listener)
  }

  rawListeners(event: string | symbol): Listener[] {
    return this.listeners(event)
  }

  listenerCount(event: string | symbol): number {
    return this.#registered.get(event)?.length ?? 0
  }

  eventNames(): (string | symbol)[] {
    return [...this.#registered.keys()]
  }

  setMaxListeners(count: number): this {
    this.#maxListeners = count
    return this
  }

  getMaxListeners(): number {
    return this.#maxListeners
  }

  #add(
    event: string | symbol,
    listener: Listener,
    once: boolean,
    prepend: boolean,
  ): this {
    this._checkEvent(event)
    const list = this.#registered.get(event) ?? []
    const first = list.length === 0
    if (prepend) list.unshift({ listener, once })
    else list.push({ listener, once })
    this.#registered.set(event, list)
    if (first) this._listenersChanged(event, true)
    return this
  }
}
