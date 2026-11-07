/**
 * Playwright: `server/clock.ts` `Clock`
 *
 * The bundle entry `dist/clock-call.js`: `(op, ...args) => unknown`, every call neo-runtime's
 * `clock.rs` makes on the page's clock controller once `dist/clock-install.js` installed it, by
 * name, so no page code travels from Rust. It runs as an init script (`log`, which a new
 * document replays) and as `UtilityScript.evaluate`'s expression in a frame's main world, where
 * a controller's throw is the call's error.
 */

import type { ClockController } from '../vendor/playwright/injected/src/clock'
import type { Installed, Replaced } from './clock-install'

/** Neo: the controller's calls Playwright's server `Clock` makes, by name. */
const OPS = [
  'install',
  'fastForward',
  'pauseAt',
  'resume',
  'runFor',
  'setFixedTime',
  'setSystemTime',
  'log',
] as const

/** Neo: calls `op` on the page's clock controller; a document without a clock has nothing to do.
 * `parse` reads a time as Playwright's server reads it, with the page's own `Date`: the time,
 * or the message of the error Playwright's server would throw. */
export default function clockCall(op: string, ...args: unknown[]): unknown {
  const global = globalThis as unknown as Record<string, unknown>
  const clock = global.__pwClock as Installed | undefined
  if (op === 'parse') {
    try {
      return parseTime(clock?.builtins.Date ?? Date, args[0] as string | number)
    } catch (error) {
      return String((error as Error).message)
    }
  }
  if (!clock) return undefined
  if (op === 'end') {
    endRun(clock, global)
    return undefined
  }
  if (!(OPS as readonly string[]).includes(op))
    throw new Error(`neo-runtime-injected: no clock call ${op}`)
  const call = (
    clock.controller as unknown as Record<
      string,
      (...args: unknown[]) => unknown
    >
  )[op]
  // Playwright: `server/clock.ts` `Clock.install`: `__pwClock.controller.install(${timeMillis})`
  return call.apply(clock.controller, args)
}

/**
 * Deviation: `server/clock.ts` `Clock.uninstall`: `await progress.race(Promise.all(this._initScripts.map(script => script.dispose())));` Playwright's clock lives as long as its context; Neo's ends with the run, and each document gets its own timers back.
 *
 * Nothing of the clock outlives the run, however it ended: no timer still waiting on the fake
 * clock fires, its real time stops for good, and every property the install replaced is back as
 * it was before (`clock-install.ts`), including the two the controller's own `uninstall` leaves.
 */
function endRun(clock: Installed, global: Record<string, unknown>): void {
  stop(clock.controller)
  try {
    clock.controller.uninstall()
  } finally {
    for (const replaced of clock.replaced) restore(replaced)
    delete global.__pwClock
  }
}

/** Neo: the controller's state that none of its methods stops. TypeScript-private, these are
 * plain properties when the page runs it. */
type Running = {
  _timers: Map<number, unknown>
  _realTime: unknown
  _currentRealTimeTimer: { cancel(): void } | undefined
  _embedder: { setTimeout(task: () => void, timeout?: number): () => void }
}

/**
 * Neo: stops the controller for good. Its timers go, so none still waiting fires, not even the
 * next one a tick in flight would reach; its real time stops; and its embedder's `setTimeout`,
 * the only way it schedules real time, schedules nothing. That tick, or a `runFor` or
 * `fastForward` the run left running, which resumes real time on its way out, cannot start the
 * clock again.
 */
function stop(controller: ClockController): void {
  const running = controller as unknown as Running
  running._timers.clear()
  running._realTime = undefined
  running._currentRealTimeTimer?.cancel()
  running._currentRealTimeTimer = undefined
  running._embedder = { ...running._embedder, setTimeout: () => () => {} }
}

/** Neo: puts one property back as it was. One the page made unchangeable during the run keeps
 * what the page left there. */
function restore({ owner, key, descriptor }: Replaced): void {
  if (descriptor) Reflect.defineProperty(owner, key, descriptor)
  else Reflect.deleteProperty(owner, key)
}

/** Playwright: `server/clock.ts` `parseTime`: `return typeof epoch === 'number' ? epoch : parsed.getTime();` */
function parseTime(
  NativeDate: DateConstructor,
  epoch: string | number,
): number {
  if (!epoch) return 0
  const parsed = new NativeDate(epoch)
  // Playwright: `server/clock.ts` `parseTime`: `Invalid date: ${epoch}`
  if (!Number.isFinite(parsed.getTime()))
    throw new Error(`Invalid date: ${epoch}`)
  return typeof epoch === 'number' ? epoch : parsed.getTime()
}
