/**
 * Playwright: `client/disposable.ts` `DisposableStub`
 *
 * What `locator.highlight()` resolves to: `dispose()` runs its cleanup once, and a closed page
 * is no error. `Symbol.asyncDispose`, where the realm has it, makes it usable with
 * `await using`.
 */

import { isTargetClosedError } from './errors'

export class DisposableStub {
  #dispose: (() => Promise<void>) | undefined

  constructor(dispose: () => Promise<void>) {
    this.#dispose = dispose
    const asyncDispose = (Symbol as { asyncDispose?: symbol }).asyncDispose
    if (asyncDispose)
      Object.defineProperty(this, asyncDispose, {
        value: () => this.dispose(),
      })
  }

  async dispose(): Promise<void> {
    if (!this.#dispose) return
    try {
      const dispose = this.#dispose
      this.#dispose = undefined
      await dispose()
    } catch (e) {
      if (isTargetClosedError(e)) return
      throw e
    }
  }
}
