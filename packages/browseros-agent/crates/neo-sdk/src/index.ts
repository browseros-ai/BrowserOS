import { Browser } from './browser'
import { BrowserContext } from './browser-context'
import { Connection } from './connection'
import { EventPipe, onUncaughtListenerError } from './events'
import { setTestIdAttribute } from './locator'
import type { Page } from './page'
import { Playwright } from './playwright'
import { onReplyEvents } from './runtime-bridge'

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
  /** `context@default`: the run's context, which lists its tabs and opens new ones. */
  context: string
  /** `browser@default` (`browser_calls::BROWSER_GUID`): the BrowserOS the run is attached to. */
  browser: string
  /** The page's URL when the run started. */
  url: string
  /** `world::TEST_ID_ATTRIBUTE`: the attribute `getByTestId` names, the one the page's
   * injected script matches. */
  testIdAttribute: string
  /** `realm::AGENT_LINE_OFFSET`: lines the realm's `AsyncFunction` wrapper puts before the
   * agent's line 1, which call locations subtract. */
  lineOffset: number
}

/**
 * Builds the `page`, `context` and `playwright` a run's script receives, around one
 * `Connection` and one `EventPipe`. The run page is the context's first page, so
 * `context.pages()` returns this very object for it, and the context is the first of
 * `context.browser().contexts()`. `uncaught` rejects with the first listener error nobody
 * handled; neo-runtime races it with the script, so such an error ends the run as it would end a
 * Node process.
 */
export function createRun(init: RunInit): {
  page: Page
  context: BrowserContext
  playwright: Playwright
  uncaught: Promise<never>
} {
  setTestIdAttribute(init.testIdAttribute)
  const connection = new Connection(init.lineOffset)
  const pipe = new EventPipe()
  onReplyEvents((until) => pipe.deliver(until))
  onUncaughtListenerError((error) => pipe.report(error))
  const playwright = new Playwright()
  const browser = new Browser(
    init.browser,
    connection,
    pipe,
    playwright.chromium,
  )
  const context = new BrowserContext(init.context, connection, pipe)
  browser._didCreateContext(context)
  const page = context._addRunPage(init.page, init.mainFrame, init.url)
  return { page, context, playwright, uncaught: pipe.uncaught }
}
