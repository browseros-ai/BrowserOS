/**
 * Playwright: `server/clock.ts` `Clock._installIfNeeded`
 *
 * The bundle entry `dist/clock-install.js`: `(browserName) => void`, Playwright's fake clock on
 * the page's globals as `globalThis.__pwClock`, once per document. neo-runtime's `clock.rs` runs
 * it as an init script and in every frame of the current documents (main world);
 * `dist/clock-call.js` then drives its controller, and at run end puts back every property the
 * install replaced, as it was before.
 */

import type { ClockController } from '../vendor/playwright/injected/src/clock'
import { inject } from '../vendor/playwright/injected/src/clock'

/** Neo: a property the clock's install replaces, as it was before: no descriptor when absent. */
export type Replaced = {
  owner: object
  key: string
  descriptor: PropertyDescriptor | undefined
}

/** Neo: a document's clock as `__pwClock` holds it: Playwright's controller and the page's own
 * builtins (`inject`), and every property the install replaced, read just before it ran. */
export type Installed = {
  controller: ClockController
  builtins: { Date: DateConstructor }
  replaced: Replaced[]
}

/**
 * Neo: the globals Playwright's install replaces, the keys of its `platformOriginals`. Its
 * `uninstall` assigns their values back, which leaves `performance`, an accessor, a plain value;
 * run end also puts back how each was defined.
 */
const GLOBALS = [
  'setTimeout',
  'clearTimeout',
  'setInterval',
  'clearInterval',
  'requestAnimationFrame',
  'cancelAnimationFrame',
  'requestIdleCallback',
  'cancelIdleCallback',
  'Date',
  'performance',
  'Intl',
  'AbortSignal',
  'Temporal',
] as const

export default function clockInstall(browserName: string): void {
  const global = globalThis as unknown as Record<string, unknown>
  // Playwright: `server/clock.ts` `Clock._installIfNeeded`: `if (!globalThis.__pwClock)`
  if (global.__pwClock) return
  const replaced = replacedBy(global)
  const installed: Installed = {
    ...inject(globalThis as unknown as Window, browserName),
    replaced,
  }
  global.__pwClock = installed
}

/**
 * Neo: every property the install is about to replace, as it is now. Besides the globals, it
 * changes two properties in place, which its `uninstall` never puts back: the page after the run,
 * and a later run, would read the fake clock through them. (Its `mirrorDateProperties` also
 * writes `Date.prototype.toUTCString`, back as it found it.)
 */
function replacedBy(global: Record<string, unknown>): Replaced[] {
  const owners: [object, string][] = GLOBALS.map((key) => [global, key])
  const signal = global.AbortSignal
  // Playwright: `injected/src/clock.ts` `fakeAbortSignal`: `Object.defineProperty(abortSignal, 'timeout', {`
  if (typeof signal === 'function') owners.push([signal, 'timeout'])
  const event = global.Event
  // Playwright: `injected/src/clock.ts` `install`: `Object.defineProperty(Event.prototype, 'timeStamp', {`
  if (typeof event === 'function') owners.push([event.prototype, 'timeStamp'])
  return owners.map(([owner, key]) => ({
    owner,
    key,
    descriptor: Object.getOwnPropertyDescriptor(owner, key),
  }))
}
