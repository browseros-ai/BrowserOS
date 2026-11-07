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

type Installed = {
  controller: ClockController
  builtins: { Date: DateConstructor }
}

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
    endRun(clock.controller, global)
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
 * The controller puts back every global it faked, and the clock's global goes: a timer set on
 * the fake clock and still waiting does not fire.
 */
function endRun(
  controller: ClockController,
  global: Record<string, unknown>,
): void {
  controller.uninstall()
  delete global.__pwClock
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
