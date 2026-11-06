import { BrowserContext } from './browser-context'
import { Page } from './page'

/** The objects neo-runtime asks for at run start, named by their guids. */
export interface RunInit {
  page: string
  /** Unused until `BrowserContext` has a getter to answer. */
  context: string
  /** The page's URL when the run started. */
  url: string
}

/** Builds the `page` and `context` a run's script receives. */
export function createRun(init: RunInit): {
  page: Page
  context: BrowserContext
} {
  return { page: new Page(init.page, init.url), context: new BrowserContext() }
}
