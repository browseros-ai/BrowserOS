/**
 * Playwright: `client/page.ts` `BindingCall`
 *
 * One page call of an exposed binding, as neo-runtime's `bindings.rs` queues it (`bindingCall` on
 * the page, or on the context for its binding): the realm runs the agent's callback and answers
 * the page's waiting call with `resolve` or `reject`, which Rust delivers in the calling frame.
 */

import type { ApiName, Connection } from './connection'
import type { Frame } from './frame'
import {
  parseResult,
  type SerializedArgument,
  serializeArgument,
} from './js-handle'
import type { Page } from './page'

/** The binding commands, on a page or the context, and on a call's `bindingcall@` guid. */
export type BindingCommands = {
  exposeBinding: {
    params: { name: string }
    result: { disposable: { guid: string } }
  }
  resolve: {
    params: { result: SerializedArgument }
    result: Record<string, never>
  }
  reject: { params: { error: Thrown }; result: Record<string, never> }
}

/** What a callback threw: an `Error`'s fields, or any other value in the page's format. */
type Thrown =
  | { error: { message: string; name: string; stack?: string } }
  | { value: unknown }

/** A call as `bindings.rs` queues it. */
export interface BindingCallRecord {
  guid: string
  frame: { guid: string }
  page: { guid: string }
  name: string
  args: unknown[]
}

/** Playwright's `BindingSource`: where the call came from. */
export interface BindingSource {
  context: ReturnType<Page['context']>
  page: Page
  frame: Frame
}

// biome-ignore lint/suspicious/noExplicitAny: Playwright's binding callback type.
export type BindingCallback = (source: BindingSource, ...args: any[]) => any

export class BindingCall {
  readonly #connection: Connection
  readonly #record: BindingCallRecord
  readonly #frame: Frame

  constructor(connection: Connection, record: BindingCallRecord, frame: Frame) {
    this.#connection = connection
    this.#record = record
    this.#frame = frame
  }

  get name(): string {
    return this.#record.name
  }

  /** Playwright: `client/page.ts` `BindingCall.call`: `const result = await func(source, ...this._initializer.args.map(parseResult));` */
  async call(func: BindingCallback): Promise<void> {
    try {
      const page = this.#frame.page()
      const source = { context: page.context(), page, frame: this.#frame }
      const result = await func(source, ...this.#record.args.map(parseResult))
      this.#send('bindingCall.resolve', 'resolve', {
        result: serializeArgument(result),
      })
    } catch (error) {
      this.#send('bindingCall.reject', 'reject', { error: thrown(error) })
    }
  }

  #send<M extends 'resolve' | 'reject'>(
    apiName: ApiName,
    method: M,
    params: BindingCommands[M]['params'],
  ): void {
    // Playwright: `client/page.ts` `BindingCall.call`: `this._channel.resolve({ result: serializeArgument(result) }, kNoTimeout).catch(() => {});`
    this.#connection
      .sendBinding(apiName, this.#record.guid, method, params, 0)
      .catch(() => {})
  }
}

/**
 * Playwright's client `serializeError`, with a thrown value in the page's format, as every
 * Neo `SerializedArgument` carries one (`js-handle.ts` `serializeArgument`).
 * Playwright: `client/errors.ts` `serializeError`: `return { value: serializeValue(e, value => ({ fallThrough: value })) };`
 */
function thrown(error: unknown): Thrown {
  if (error instanceof Error)
    return {
      error: { message: error.message, name: error.name, stack: error.stack },
    }
  try {
    return { value: serializeArgument(error).value }
  } catch {
    return { value: { v: 'undefined' } }
  }
}
