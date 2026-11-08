/**
 * Playwright: `client/clock.ts` `Clock`
 *
 * `page.clock` and `browserContext.clock`, one clock for the run's context: Playwright's client,
 * sending the context's `clock*` commands (neo-runtime `clock.rs`, Playwright's server `Clock`).
 */

import type { ApiName, Connection } from './connection'

type Time = { timeNumber?: number; timeString?: string }
type Ticks = { ticksNumber?: number; ticksString?: string }

/** The clock commands, on the context. */
export type ClockCommands = {
  clockInstall: { params: Time; result: Record<string, never> }
  clockFastForward: { params: Ticks; result: Record<string, never> }
  clockPauseAt: { params: Time; result: Record<string, never> }
  clockResume: { params: Record<string, never>; result: Record<string, never> }
  clockRunFor: { params: Ticks; result: Record<string, never> }
  clockSetFixedTime: { params: Time; result: Record<string, never> }
  clockSetSystemTime: { params: Time; result: Record<string, never> }
}

export class Clock {
  readonly #connection: Connection
  readonly #guid: string

  constructor(connection: Connection, guid: string) {
    this.#connection = connection
    this.#guid = guid
  }

  async install(
    options: { time?: number | string | Date } = {},
  ): Promise<void> {
    await this.#send(
      'clock.install',
      'clockInstall',
      options.time !== undefined
        ? parseTime('clock.install', options.time)
        : {},
    )
  }

  async fastForward(ticks: number | string): Promise<void> {
    await this.#send(
      'clock.fastForward',
      'clockFastForward',
      parseTicks('clock.fastForward', ticks),
    )
  }

  async pauseAt(time: number | string | Date): Promise<void> {
    await this.#send(
      'clock.pauseAt',
      'clockPauseAt',
      parseTime('clock.pauseAt', time),
    )
  }

  async resume(): Promise<void> {
    await this.#send('clock.resume', 'clockResume', {})
  }

  async runFor(ticks: number | string): Promise<void> {
    await this.#send(
      'clock.runFor',
      'clockRunFor',
      parseTicks('clock.runFor', ticks),
    )
  }

  async setFixedTime(time: string | number | Date): Promise<void> {
    await this.#send(
      'clock.setFixedTime',
      'clockSetFixedTime',
      parseTime('clock.setFixedTime', time),
    )
  }

  async setSystemTime(time: string | number | Date): Promise<void> {
    await this.#send(
      'clock.setSystemTime',
      'clockSetSystemTime',
      parseTime('clock.setSystemTime', time),
    )
  }

  async #send<M extends keyof ClockCommands>(
    apiName: ApiName,
    method: M,
    params: ClockCommands[M]['params'],
  ): Promise<void> {
    // Playwright: `client/clock.ts` `Clock.install`: `await this._browserContext._channel.clockInstall({ ...(options.time !== undefined ? parseTime(options.time) : {}) }, kNoTimeout);`
    await this.#connection.sendClock(apiName, this.#guid, method, params, 0)
  }
}

/** Playwright: `client/clock.ts` `parseTime`: `return { timeNumber: time.getTime() };` */
function parseTime(apiName: ApiName, time: string | number | Date): Time {
  if (typeof time === 'number') return { timeNumber: number(apiName, time) }
  if (typeof time === 'string') return { timeString: time }
  // Playwright: `client/clock.ts` `parseTime`: `Invalid date: ${time}`
  if (!Number.isFinite(time.getTime())) throw new Error(`Invalid date: ${time}`)
  return { timeNumber: time.getTime() }
}

/** Playwright: `client/clock.ts` `parseTicks`: `ticksNumber: typeof ticks === 'number' ? ticks : undefined,` */
function parseTicks(apiName: ApiName, ticks: string | number): Ticks {
  if (typeof ticks === 'string') return { ticksString: ticks }
  if (!Number.isFinite(ticks))
    // Deviation: `server/clock.ts` `parseTicks`: `Invalid ticks: ${value}` Playwright's server reads a NaN or an infinite count, which Neo's JSON to Rust cannot carry; the SDK refuses it as the server does.
    throw new Error(`${apiName}: Invalid ticks: ${ticks}`)
  return { ticksNumber: ticks }
}

/**
 * Neo: a time that is a number, as Playwright's server `parseTime` reads one Neo's JSON cannot
 * carry: `NaN` is no time (`0`), an infinite one is no date.
 */
function number(apiName: ApiName, time: number): number {
  if (Number.isNaN(time)) return 0
  if (!Number.isFinite(time))
    throw new Error(`${apiName}: Invalid date: ${time}`)
  return time
}
