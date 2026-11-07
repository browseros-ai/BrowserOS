/**
 * Playwright: `client/worker.ts` `Worker`
 *
 * A dedicated worker of a page (neo-runtime `workers.rs`): its URL, its evaluations, which run
 * in the worker's own execution context (`worker_eval.rs`), and its `close`. The page builds one
 * object per guid (`Page._workerFrom`), from its `worker` event or from `page.workers()`, and
 * routes the worker's close to it (`workerClose`), which the page listens for from its first
 * look at its workers.
 *
 * ```text
 *  page.on('worker') or page.workers() ─▶ Rust tracks the tab's workers (auto-attach)
 *  worker attaches ─▶ 'worker' {worker: {guid, initializer: {url}}} ─▶ Page._workerFrom
 *  worker.evaluate(fn) ─▶ {guid: 'worker@T:1', method: 'evaluateExpression'} ─▶ worker_eval.rs
 *  worker detaches, or its page closes ─▶ 'workerClose' {worker: {guid}} ─▶ worker 'close'
 * ```
 *
 * Deviation: `client/worker.ts` `Worker.constructor`: `this.emit(Events.Worker.Console, new ConsoleMessage(event, null, this));`
 * Neo reports no worker's console yet: a `console` listener on a worker throws, as every event
 * Neo does not produce does.
 */

import type { WorkerInitializer } from '@browseros/neo-runtime-spec/serve'
import type { Connection } from './connection'
import { type PlaywrightError, targetClosedError } from './errors'
import { ChannelOwner, type EventPipe, type EventTable } from './events'
import {
  assertMaxArguments,
  evaluateParams,
  handleFrom,
  type JSHandle,
  type PageFunction,
  parseResult,
} from './js-handle'
import type { Page } from './page'
import { Waiter, type WaitForEventOptions, wrapApiCall } from './waiter'

/** Neo: a worker's listener events (`Events.Worker`): its `close`, which the page routes to it,
 * and its `console`, which Neo does not produce yet. */
const WORKER_EVENTS: EventTable = {
  receiver: 'Worker',
  served: [],
  local: ['close'],
  quiet: [],
  unserved: ['console'],
}

/** Neo: a worker record Rust sends (`workers.rs` `Worker::record`): Playwright's
 * `{guid, initializer}`. */
export interface CreatedWorker {
  guid: string
  initializer: WorkerInitializer
}

export class Worker extends ChannelOwner {
  readonly #initializer: WorkerInitializer
  readonly #page: Page
  readonly #connection: Connection

  constructor(
    record: CreatedWorker,
    page: Page,
    pipe: EventPipe,
    connection: Connection,
  ) {
    super(record.guid, pipe, WORKER_EVENTS)
    this.#initializer = record.initializer
    this.#page = page
    this.#connection = connection
  }

  /** Rust sends nothing on a worker's guid: its page routes its close (`_onClose`). */
  _onEvent(_method: string, _params: unknown): void {}

  /** Playwright: `client/worker.ts` `Worker.constructor`: `this.emit(Events.Worker.Close, this);` */
  _onClose(): void {
    this.emit('close', this)
  }

  /** Playwright: `client/worker.ts` `Worker._closeErrorWithReason`: `return this._page?._closeErrorWithReason() || new TargetClosedError();` */
  _closeErrorWithReason(): PlaywrightError {
    return targetClosedError(this.#page._closeReason())
  }

  url(): string {
    return this.#initializer.url
  }

  /** Runs `pageFunction` in the worker and resolves with its value, with no timeout
   * (Playwright's `kNoTimeout`). */
  async evaluate<R, Arg>(
    pageFunction: PageFunction<Arg, R>,
    arg?: Arg,
  ): Promise<R> {
    // biome-ignore lint/complexity/noArguments: Playwright counts the call's arguments.
    assertMaxArguments(arguments.length, 2)
    const params = evaluateParams(
      'worker.evaluate',
      pageFunction,
      arg,
      undefined,
    )
    const result = await this.#connection.sendEval(
      'worker.evaluate',
      this._guid,
      'evaluateExpression',
      params,
      0,
    )
    return parseResult(result.value) as R
  }

  /** {@link Worker.evaluate}, resolving with a handle on the worker's value. */
  async evaluateHandle<R, Arg>(
    pageFunction: PageFunction<Arg, R>,
    arg?: Arg,
  ): Promise<JSHandle<R>> {
    // biome-ignore lint/complexity/noArguments: Playwright counts the call's arguments.
    assertMaxArguments(arguments.length, 2)
    const params = evaluateParams(
      'worker.evaluateHandle',
      pageFunction,
      arg,
      undefined,
    )
    const result = await this.#connection.sendEval(
      'worker.evaluateHandle',
      this._guid,
      'evaluateExpressionHandle',
      params,
      0,
    )
    return handleFrom(this.#connection, result.handle) as JSHandle<R>
  }

  /**
   * Playwright's `Worker.waitForEvent`: resolves with the first `event` its predicate accepts,
   * under the page's timeout, and rejects when the worker closes first.
   */
  async waitForEvent(
    event: string,
    optionsOrPredicate: WaitForEventOptions = {},
  ): Promise<unknown> {
    return await wrapApiCall('worker.waitForEvent', async () => {
      const options =
        typeof optionsOrPredicate === 'function' ? {} : optionsOrPredicate
      const timeoutOptions = this.#page._timeoutSettings.timeout(options, null)
      this._refuseUnserved(event)
      const predicate =
        typeof optionsOrPredicate === 'function'
          ? optionsOrPredicate
          : optionsOrPredicate.predicate
      const waiter = Waiter.createForEvent()
      waiter.rejectOnTimeout(
        timeoutOptions,
        `Timeout ${timeoutOptions.timeout}ms exceeded while waiting for event "${event}"`,
      )
      if (event !== 'close')
        waiter.rejectOnEvent(this, 'close', () => this._closeErrorWithReason())
      const result = await waiter.waitForEvent(this, event, predicate)
      waiter.dispose()
      return result
    })
  }
}
