/**
 * Playwright: `server/dom.ts` `FrameExecutionContext.injectedScript`
 *
 * The bundle entry `dist/main-injected.js`: `(initializer) => InjectedScript` for the page's own
 * (main) world, built as Playwright builds that world's injected script (`isUtilityWorld:
 * false`). neo-runtime's `main_world.rs` keeps the object by CDP id in its own object group and
 * hands it to the built element callbacks (`dist/call-on-element.js`). Nothing is stored on the
 * page's globals, unlike `install.ts` in Neo's isolated world, and nothing is left on the page
 * once the run ends ({@link MainWorldInjectedScript}).
 */

import type { InjectedInitializer } from '@browseros/neo-playwright-protocol/injected'
import {
  InjectedScript,
  type InjectedScriptOptions,
} from '../vendor/playwright/injected/src/injectedScript'

/** Neo: the constructor's two setups that add window listeners and a document observer. */
const SKIPPED_SETUPS = [
  '_setupGlobalListenersRemovalDetection',
  '_setupHitTargetInterceptors',
] as const

/**
 * Playwright's `InjectedScript`, without the window listeners and the document observer its
 * constructor adds for the hit-target interceptor. Only `setupHitTargetInterceptor` uses them,
 * and Playwright runs that in the utility world. Playwright builds one main-world instance per
 * context; Neo builds one per run and document, and each would stay on the page after its run,
 * kept alive by those listeners.
 * Deviation: `injected/src/injectedScript.ts` `InjectedScript.constructor`: `this._setupHitTargetInterceptors();` Neo's main-world instance adds no window listener or document observer.
 */
class MainWorldInjectedScript extends InjectedScript {}

/**
 * Neo: makes each of {@link SKIPPED_SETUPS} do nothing on `prototype`. A vendored Playwright
 * that renamed one fails every main-world call here, where the override would quietly stop
 * applying and the listeners come back.
 */
function skipSetups(prototype: object): void {
  for (const setup of SKIPPED_SETUPS) {
    const own = (
      InjectedScript.prototype as unknown as Record<string, unknown>
    )[setup]
    if (typeof own !== 'function')
      throw new Error(`neo-playwright-injected: InjectedScript has no ${setup}`)
    Object.defineProperty(prototype, setup, { value: () => {} })
  }
}
skipSetups(MainWorldInjectedScript.prototype)

export default function mainInjected(
  initializer: InjectedInitializer,
): InjectedScript {
  const fromSpec: Pick<InjectedScriptOptions, keyof InjectedInitializer> =
    initializer
  return new MainWorldInjectedScript(window, {
    ...fromSpec,
    isUnderTest: false,
    sdkLanguage: 'javascript',
    browserName: 'chromium',
    shouldPrependErrorPrefix: false,
    customEngines: [],
  })
}
