/**
 * Playwright: `server/dom.ts` `FrameExecutionContext.injectedScript`
 *
 * The bundle entry `dist/main-injected.js`: `(initializer) => InjectedScript` for the page's own
 * (main) world, built as Playwright builds that world's injected script (`isUtilityWorld:
 * false`). neo-runtime's `main_world.rs` keeps the object by CDP id in its own object group and
 * hands it to the built element callbacks (`dist/call-on-element.js`). Nothing is stored on the
 * page's globals, unlike `install.ts` in Neo's isolated world.
 */

import type { InjectedInitializer } from '@browseros/neo-runtime-spec/injected'
import {
  InjectedScript,
  type InjectedScriptOptions,
} from '../vendor/playwright/injected/src/injectedScript'

export default function mainInjected(
  initializer: InjectedInitializer,
): InjectedScript {
  const fromSpec: Pick<InjectedScriptOptions, keyof InjectedInitializer> =
    initializer
  return new InjectedScript(window, {
    ...fromSpec,
    isUnderTest: false,
    sdkLanguage: 'javascript',
    browserName: 'chromium',
    shouldPrependErrorPrefix: false,
    customEngines: [],
  })
}
