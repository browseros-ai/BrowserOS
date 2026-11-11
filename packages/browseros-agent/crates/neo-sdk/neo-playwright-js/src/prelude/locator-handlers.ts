/**
 * `page.addLocatorHandler` and `removeLocatorHandler`, following playwright-core 1.64.1's
 * client/page.ts: the script registers a function for an overlay that may appear at any time,
 * and an action that finds the overlay visible pauses while the script runs it.
 *
 *   page.addLocatorHandler(l, fn)   page.addLocatorHandler { target, noWaitAfter } ─▶ { uid }
 *   an action's retry, in Rust      l is visible ─ queue uid, pause the action
 *   pull (one per run)              page.locatorHandlerTriggered {}, a background long poll
 *                                   ─▶ { uid } ─▶ fn(l), counting `times`
 *                                   ─▶ page.resolveLocatorHandler { uid, remove }
 *                                   ─ the action waits for l to be hidden, then retries
 *   page.removeLocatorHandler(l)    page.removeLocatorHandler { uid }, per matching registration
 *
 * Rust owns the checkpoint (crates/neo-sdk/neo-playwright/src/locator_handler.rs), as upstream's
 * server does, so every retry of every action sees a handler registered while it waited. Nothing
 * calls into the runtime from Rust (events.ts): the realm hears of a trigger only because the
 * pull asks. The pull answers `{ uid: null }` once the run has no handler left, and the host cuts
 * it quietly when the run ends, as it cuts the event pump. Handlers live as long as the run: the
 * functions they call live in its realm.
 */
import { invoke } from './bridge'
import { plainError } from './errors'
import { Locator } from './locator'
import type { Page } from './page'

/** Upstream's client-side record of one registration. */
type Registration = {
  page: Page
  locator: Locator
  handler: (locator: Locator) => unknown
  /** How many more runs; undefined runs it every time. */
  times: number | undefined
}

export type LocatorHandlerOptions = { times?: number; noWaitAfter?: boolean }

/** The run's locator handlers, by the uid Rust gave each registration. One per context. */
export class LocatorHandlers {
  readonly #byUid = new Map<number, Registration>()
  #pulling = false

  async add(
    api: string,
    page: Page,
    locator: Locator,
    handler: (locator: Locator) => unknown,
    options: LocatorHandlerOptions = {},
  ): Promise<void> {
    if (!(locator instanceof Locator) || locator._frame !== page._mainFrame)
      throw plainError(
        api,
        'Locator must belong to the main frame of this page',
      )
    if (options.times === 0) return
    const target = await locator._frame._locatorBrand(api, locator._selector)
    const { uid } = await invoke(
      'page.addLocatorHandler',
      { api, timeoutMs: 0 },
      options.noWaitAfter ? { target, noWaitAfter: true } : { target },
    )
    this.#byUid.set(uid, { page, locator, handler, times: options.times })
    if (!this.#pulling) {
      this.#pulling = true
      void this.pull()
    }
  }

  /** Every registration of `locator` on `page`, as upstream compares locators: frame and selector. */
  async remove(api: string, page: Page, locator: Locator): Promise<void> {
    for (const [uid, registration] of this.#byUid) {
      if (
        registration.page !== page ||
        !(locator instanceof Locator) ||
        registration.locator._frame !== locator._frame ||
        registration.locator._selector !== locator._selector
      )
        continue
      this.#byUid.delete(uid)
      await invoke(
        'page.removeLocatorHandler',
        { api, timeoutMs: 0 },
        { uid },
      ).catch(() => {})
    }
  }

  private async pull(): Promise<void> {
    try {
      while (this.#byUid.size > 0) {
        const { uid } = await invoke(
          'page.locatorHandlerTriggered',
          { api: 'page.locatorHandlerTriggered', timeoutMs: 0 },
          {},
        )
        // Not awaited: a handler that waits on another tab's action must not hold back a
        // trigger that action raises. A throwing handler surfaces as an unhandled rejection,
        // as upstream's does.
        if (uid !== null) void this.#run(uid)
      }
    } catch {
      // The host cuts the pull quietly when the run ends; one that failed otherwise starts
      // again with the next registration.
    } finally {
      this.#pulling = false
    }
  }

  /** Upstream's `_onLocatorHandlerTriggered`: always resolves, so the paused action goes on. */
  async #run(uid: number): Promise<void> {
    let remove = false
    try {
      const registration = this.#byUid.get(uid)
      if (registration && registration.times !== 0) {
        if (registration.times !== undefined) registration.times--
        await registration.handler(registration.locator)
      }
      remove = registration?.times === 0
    } finally {
      if (remove) this.#byUid.delete(uid)
      await invoke(
        'page.resolveLocatorHandler',
        { api: 'page.resolveLocatorHandler', timeoutMs: 0 },
        remove ? { uid, remove: true } : { uid },
      ).catch(() => {})
    }
  }
}
