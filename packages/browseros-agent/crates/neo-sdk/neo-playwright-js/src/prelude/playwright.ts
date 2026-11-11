/**
 * The agent's `playwright` parameter: the part of Playwright's module object a script in one
 * Neo profile can use. A script written for Playwright reaches selectors and error classes
 * through it (`playwright.selectors.setTestIdAttribute`, `playwright.errors.TimeoutError`); the
 * browser types, `devices` and `request` stay out, since Neo launches no browser and its one
 * profile takes no device options.
 */
import { internalError } from './errors'
import { localTimeout } from './host'
import { setTestIdAttribute } from './locator'

/**
 * Upstream's `Selectors` (client/selectors.ts at v1.64.1). `setTestIdAttribute` changes what
 * `getByTestId` builds for the rest of this run. `register` is refused (unsupported.ts): a
 * custom engine would have to be installed into every frame's engine and again after each
 * navigation.
 */
export class Selectors {
  setTestIdAttribute(attributeName: string): void {
    setTestIdAttribute(attributeName)
  }
}

/** Error classes as upstream's `playwright.errors` exposes them. */
export type Errors = { TimeoutError: new (message: string) => Error }

/**
 * The run's `playwright`. `errors.TimeoutError` is the bootstrap's own class, the one every Neo
 * timeout is an instance of, so `error instanceof playwright.errors.TimeoutError` holds.
 */
export function createPlaywright(): { selectors: Selectors; errors: Errors } {
  const sample = localTimeout('')
  if (sample === null)
    throw internalError(
      'playwright',
      "claw-sdk's bootstrap has no TimeoutError",
    )
  return {
    selectors: new Selectors(),
    errors: { TimeoutError: sample.constructor as Errors['TimeoutError'] },
  }
}
