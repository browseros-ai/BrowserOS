import { BrowserContext } from './browser-context'
import { setTestIdAttribute } from './locator'
import { Page } from './page'

/**
 * What neo-runtime hands `createRun` at run start (`realm.rs` `start`). Every value comes from
 * a Rust constant or the run's tab, so the SDK derives nothing on its own.
 */
export interface RunInit {
  /** `page@<targetId>`. */
  page: string
  /** `frame@<targetId>`: the page's main frame. Chromium gives a page's main frame its
   * target's id. */
  mainFrame: string
  /** Unused until `BrowserContext` serves a browser call. */
  context: string
  /** The page's URL when the run started. */
  url: string
  /** `world::TEST_ID_ATTRIBUTE`: the attribute `getByTestId` names, the one the page's
   * injected script matches. */
  testIdAttribute: string
}

/** Builds the `page` and `context` a run's script receives. */
export function createRun(init: RunInit): {
  page: Page
  context: BrowserContext
} {
  setTestIdAttribute(init.testIdAttribute)
  const context = new BrowserContext()
  const page = new Page(init.page, init.mainFrame, init.url, context)
  return { page, context }
}
