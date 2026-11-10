/**
 * The Node-style emitter playwright-core's classes extend, so `page.on`, `once`, `off` and the
 * rest behave as scripts expect. Subclasses learn when an event gains its first listener or
 * loses its last one (`_listenersChanged`), which is how Page and BrowserContext tell Rust which
 * events this run wants, and may refuse an event Neo never reports (`_checkEvent`).
 */
export type Listener = (...args: never[]) => unknown

type Registered = { listener: Listener; once: boolean }

export class EventEmitter {
  readonly #registered = new Map<string | symbol, Registered[]>()
  #maxListeners = 10

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

  removeAllListeners(event?: string | symbol): this {
    const events = event === undefined ? [...this.#registered.keys()] : [event]
    for (const name of events) {
      if (!this.#registered.delete(name)) continue
      this._listenersChanged(name, false)
    }
    return this
  }

  emit(event: string | symbol, ...args: unknown[]): boolean {
    const list = this.#registered.get(event)
    if (!list || list.length === 0) return false
    for (const entry of [...list]) {
      if (entry.once) this.removeListener(event, entry.listener)
      // A throwing listener must not stop the others or the event pump that called emit; its
      // error surfaces as an unhandled rejection, as an uncaught listener error does in Node.
      try {
        Reflect.apply(entry.listener, this, args)
      } catch (error) {
        void Promise.reject(error)
      }
    }
    return true
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
