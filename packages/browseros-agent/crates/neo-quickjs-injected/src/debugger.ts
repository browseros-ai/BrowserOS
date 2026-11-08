/**
 * Playwright: `client/debugger.ts` `Debugger`
 *
 * Playwright's `context.debugger`: `requestPause()` makes the context's next pausable call wait
 * before it starts, and `pausedDetails()` names it (its location in the script and its title)
 * until `resume()`, `next()` (stop at the call after) or `runTo(location)` lets it go. A Neo run
 * is the only client of its browser, so the calls it pauses are its own: a script pauses a call
 * it has not awaited yet, and resumes it from another branch. `page.pause()` stops here too once
 * the debugger is enabled.
 *
 * Deviation: `client/debugger.ts` `Debugger.constructor`: `this._pausedDetails = pausedDetails ?? null;`
 * Playwright's client keeps the details its server sent last; Neo reads them from Rust at each
 * call, so they are current even when nothing listens to `pausedstatechanged`.
 */

import type { Connection } from './connection'
import { ChannelOwner, type EventPipe, type EventTable } from './events'
import { get } from './runtime-bridge'

/**
 * Neo: the debugger's listener events (`Events.Debugger`). `apicallsupdated` streams calls to a
 * debugging client (Playwright's internal `_enable`), which Neo has none of.
 */
const DEBUGGER_EVENTS: EventTable = {
  receiver: 'Debugger',
  served: ['pausedstatechanged'],
  quiet: [],
  unserved: ['apicallsupdated'],
}

/** Playwright's `PausedDetails`: where the held call was made, and what it is. */
export interface PausedDetails {
  location: { file: string; line?: number; column?: number }
  title: string
  stack?: string
}

export class Debugger extends ChannelOwner {
  readonly #connection: Connection

  constructor(guid: string, connection: Connection, pipe: EventPipe) {
    super(guid, pipe, DEBUGGER_EVENTS)
    this.#connection = connection
  }

  /** Playwright's debugger channel handler: `pausedStateChanged`. */
  _onEvent(method: string): void {
    if (method === 'pausedStateChanged') this.emit('pausedstatechanged')
  }

  async requestPause(): Promise<void> {
    await this.#connection.sendDebugger(
      'debugger.requestPause',
      this._guid,
      'requestPause',
      {},
      0,
    )
  }

  async resume(): Promise<void> {
    await this.#connection.sendDebugger(
      'debugger.resume',
      this._guid,
      'resume',
      {},
      0,
    )
  }

  async next(): Promise<void> {
    await this.#connection.sendDebugger(
      'debugger.next',
      this._guid,
      'next',
      {},
      0,
    )
  }

  async runTo(location: {
    file: string
    line?: number
    column?: number
  }): Promise<void> {
    await this.#connection.sendDebugger(
      'debugger.runTo',
      this._guid,
      'runTo',
      { location },
      0,
    )
  }

  /** The call the debugger holds now, or `null` while it holds none. */
  pausedDetails(): PausedDetails | null {
    return get(this._guid, 'pausedDetails')
  }
}
