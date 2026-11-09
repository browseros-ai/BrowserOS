import type { BrowserContext } from './browser-context'
import { exportValue } from './buffer'
import { standardGlobals } from './globals'
import type { Page } from './page'
import type { Playwright } from './playwright'
import { connectRun, type RunInit } from './run-objects'

export { createHooks, type HooksInit } from './hooks'
export type { RunInit } from './run-objects'

/**
 * Builds the `page`, `context` and `playwright` a run's script receives, around one
 * `Connection` and one `EventPipe` (`run-objects.ts` `connectRun`). The run page is the
 * context's first page, so `context.pages()` returns this very object for it, and the context
 * is the first of `context.browser().contexts()`. `uncaught` rejects with the first listener
 * error nobody handled; neo-runtime races it with the script, so such an error ends the run as
 * it would end a Node process. `exportValue` is how neo-runtime turns the script's return value
 * into JSON (`buffer.ts`). `globals` are the standard globals (`URL`, `Buffer`, …) neo-runtime
 * defines before the script runs (`globals.ts`).
 */
export function createRun(init: RunInit): {
  page: Page
  context: BrowserContext
  playwright: Playwright
  uncaught: Promise<never>
  exportValue: typeof exportValue
  globals: Record<string, unknown>
} {
  const { page, context, playwright, pipe } = connectRun(init)
  return {
    page,
    context,
    playwright,
    uncaught: pipe.uncaught,
    exportValue,
    globals: standardGlobals(),
  }
}
