/**
 * Playwright: `server/clock.ts` `Clock._installIfNeeded`
 *
 * The bundle entry `dist/clock-install.js`: `(browserName) => void`, Playwright's fake clock on
 * the page's globals as `globalThis.__pwClock`, once per document. neo-runtime's `clock.rs` runs
 * it as an init script and in every frame of the current documents (main world);
 * `dist/clock-call.js` then drives its controller.
 */

import { inject } from '../vendor/playwright/injected/src/clock'

export default function clockInstall(browserName: string): void {
  const global = globalThis as unknown as Record<string, unknown>
  // Playwright: `server/clock.ts` `Clock._installIfNeeded`: `if (!globalThis.__pwClock)`
  if (!global.__pwClock)
    global.__pwClock = inject(globalThis as unknown as Window, browserName)
}
