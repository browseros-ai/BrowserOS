/** One dedicated worker target. Page facts and event records share this object; a close is
 * terminal even when an older worker-created record arrives after a newer host observation. */
import { invoke, type WorkerInfo } from './bridge'
import { captureSite } from './call-site'
import { EventEmitter } from './emitter'
import { clientError, unsupportedError } from './errors'
import { assertMaxArguments, fromPageValue, pageCall } from './evaluation'
import { EventDeadline } from './event-deadline'
import type { ConsoleMessage } from './event-values'
import { markHandle } from './options'
import type { Page } from './page'
import { timeoutMs } from './timeouts'

export class Worker extends EventEmitter {
  readonly #page: Page
  readonly #info: WorkerInfo
  _closed = false
  #consoleForward: ((message: ConsoleMessage) => void) | null = null

  constructor(page: Page, info: WorkerInfo) {
    super()
    this.#page = page
    this.#info = info
    markHandle(this)
  }

  url(): string {
    return this.#info.url
  }

  async evaluate(pageFunction: unknown, arg?: unknown): Promise<unknown> {
    const site = captureSite()
    const api = 'worker.evaluate'
    assertMaxArguments(api, arguments.length, 2)
    const script = pageCall(api, 'frame', pageFunction, arg)
    if (this._closed)
      throw clientError(api, 'Target page, context or browser has been closed')
    const { value } = await invoke(
      'worker.evaluate',
      site,
      { api, timeoutMs: 0 },
      { page: await this.#page._brand(api), worker: this.#info.id, script },
    )
    return fromPageValue(api, value)
  }

  evaluateHandle(): never {
    throw unsupportedError(
      'worker.evaluateHandle',
      'JS handles are not available in Neo; use worker.evaluate instead',
    )
  }

  async waitForEvent(
    event: string,
    optionsOrPredicate: unknown = {},
  ): Promise<unknown> {
    const api = 'worker.waitForEvent'
    if (event !== 'close' && event !== 'console')
      throw unsupportedError(
        api,
        `Worker event "${event}" is not available in Neo`,
      )
    const object =
      typeof optionsOrPredicate === 'object' && optionsOrPredicate !== null
        ? optionsOrPredicate
        : {}
    const predicate: unknown =
      typeof optionsOrPredicate === 'function'
        ? optionsOrPredicate
        : Reflect.get(object, 'predicate')
    const timeout: unknown = Reflect.get(object, 'timeout')
    if (predicate !== undefined && typeof predicate !== 'function')
      throw clientError(api, 'predicate: expected function')
    if (timeout !== undefined && typeof timeout !== 'number')
      throw clientError(api, 'timeout: expected number')
    const options = timeout === undefined ? {} : { timeout }
    const deadline = new EventDeadline(
      api,
      event,
      Date.now(),
      timeoutMs(api, this.#page._timeoutSettings.timeout(options)),
    )
    let listener: (value: never) => void = () => {}
    let closed: () => void = () => {}
    const result = new Promise<unknown>((resolve, reject) => {
      listener = (value) => {
        if (typeof predicate !== 'function') {
          resolve(value)
          return
        }
        try {
          const accepted: unknown = predicate(value)
          if (
            (typeof accepted === 'object' || typeof accepted === 'function') &&
            accepted !== null &&
            typeof Reflect.get(accepted, 'then') === 'function'
          ) {
            void Promise.resolve(accepted).then((result) => {
              if (result) resolve(value)
            }, reject)
          } else if (accepted) resolve(value)
        } catch (error) {
          reject(error)
        }
      }
      closed = () =>
        reject(
          clientError(api, 'Target page, context or browser has been closed'),
        )
      this.on(event, listener)
      if (event !== 'close') this.on('close', closed)
      if (this._closed) closed()
    })
    try {
      return await deadline.race(result)
    } finally {
      this.off(event, listener)
      this.off('close', closed)
    }
  }

  /** A terminal transition is delivered once, through either facts or the event pump. */
  _didClose(): void {
    if (this._closed) return
    this._closed = true
    if (this.#consoleForward) this.#page.off('console', this.#consoleForward)
    this.emit('close', this)
  }

  protected override _listenersChanged(
    event: string | symbol,
    listening: boolean,
  ): void {
    if (event !== 'console') return
    if (listening && !this._closed) {
      this.#consoleForward = (message) => {
        if (message.worker() === this) this.emit('console', message)
      }
      this.#page.on('console', this.#consoleForward)
    } else if (this.#consoleForward) {
      this.#page.off('console', this.#consoleForward)
      this.#consoleForward = null
    }
  }
}
