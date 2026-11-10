/**
 * Playwright: `server/page.ts` `PageBinding.createInitScript`
 *
 * The bundle entry `dist/bindings-controller.js`: `() => void`, the page half of
 * `exposeBinding`. It installs Playwright's `BindingsController` as the page's
 * `globalThis.__playwright__binding__controller__`, once per document, as Playwright's
 * bindings init script does. neo-runtime's `bindings.rs` runs it as an init script and in every
 * frame of the page's current document (main world); `dist/bindings-call.js` then adds, removes
 * and answers bindings through it. The controller calls the CDP binding
 * `__playwright__binding__`, which `Runtime.addBinding` puts on each of the run's sessions.
 */

import { BindingsController } from '../vendor/playwright/injected/src/bindingsController'
import { kBindingsControllerProperty } from '../vendor/playwright/isomorphic/utilityScriptSerializers'

/** Playwright: `server/page.ts` `PageBinding.kBindingName`: `static kBindingName = '__playwright__binding__';` */
const BINDING_NAME = '__playwright__binding__'

export default function bindingsController(): void {
  const global = globalThis as unknown as Record<string, unknown>
  // Playwright: `server/page.ts` `PageBinding.createInitScript`: `if (!globalThis[property])`
  if (!global[kBindingsControllerProperty])
    global[kBindingsControllerProperty] = new BindingsController(
      globalThis,
      BINDING_NAME,
    )
}
