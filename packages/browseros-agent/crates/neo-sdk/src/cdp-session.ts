/**
 * Playwright: `client/cdpSession.ts` `CDPSession`
 *
 * `context.newCDPSession(page | frame)`: a CDP session of the script's own on the page's (or an
 * out-of-process frame's) target. neo-runtime forwards what `send` asks, guarded
 * (`cdp_session.rs` `REFUSED`), and hands the session's CDP events back as Playwright's: each
 * under its own method name, every one as `event`, and `close` once the session detached.
 */

import type {
  CDPSessionSendParams,
  CDPSessionSendResult,
} from '@browseros/neo-runtime-spec/serve'
import type { Connection } from './connection'
import { ChannelOwner, type EventPipe, type EventTable } from './events'
import type { TimeoutInput } from './timeout-settings'

// biome-ignore lint/suspicious/noExplicitAny: an emitter listener takes whatever its event carries.
type Listener = (...args: any[]) => any
type EventType = string | symbol

/** Neo: a session's channel events. `close` is gated as any served event; `event`, which every
 * CDP method name a script listens to reads, is gated by {@link CDPSession}'s listeners. */
const CDP_SESSION_EVENTS: EventTable = {
  receiver: 'CDPSession',
  served: ['close'],
  quiet: [],
  unserved: [],
}

export class CDPSession extends ChannelOwner {
  readonly #connection: Connection
  #listening = false

  constructor(guid: string, connection: Connection, pipe: EventPipe) {
    super(guid, pipe, CDP_SESSION_EVENTS)
    this.#connection = connection
  }

  _onEvent(method: string, params: unknown): void {
    if (method === 'event') {
      const event = params as { method: string; params?: unknown }
      this.emit(event.method, event.params)
      this.emit('event', event)
    } else if (method === 'close') {
      this.emit('close', this)
    }
  }

  /** Neo: the session's events arrive under any CDP method name, so the channel's `event` is on
   * while any listener but `close`'s is. */
  #update(): void {
    const listening = this.eventNames().some(
      (name) => name !== 'close' && this.listenerCount(name) > 0,
    )
    if (listening === this.#listening) return
    this.#listening = listening
    this._subscribeChannelEvent('event', listening)
  }

  override on(event: EventType, listener: Listener): this {
    super.on(event, listener)
    this.#update()
    return this
  }

  override addListener(event: EventType, listener: Listener): this {
    super.addListener(event, listener)
    this.#update()
    return this
  }

  override prependListener(event: EventType, listener: Listener): this {
    super.prependListener(event, listener)
    this.#update()
    return this
  }

  override off(event: EventType, listener: Listener): this {
    super.off(event, listener)
    this.#update()
    return this
  }

  override removeListener(event: EventType, listener: Listener): this {
    super.removeListener(event, listener)
    this.#update()
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
    const result = options
      ? super.removeAllListeners(type, options)
      : super.removeAllListeners(type)
    this.#update()
    return result
  }

  async send(
    method: string,
    params?: CDPSessionSendParams['params'],
    options: TimeoutInput = {},
  ): Promise<CDPSessionSendResult['result']> {
    const result = await this.#connection.sendCDPSession(
      'cdpSession.send',
      this._guid,
      'send',
      params === undefined ? { method } : { method, params },
      { signal: options.signal, timeout: options.timeout ?? 0 },
    )
    return result.result
  }

  async detach(): Promise<void> {
    await this.#connection.sendCDPSession(
      'cdpSession.detach',
      this._guid,
      'detach',
      {},
      0,
    )
  }
}
