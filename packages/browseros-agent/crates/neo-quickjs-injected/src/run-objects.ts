/**
 * Neo: the browser objects one realm's code drives, built the same way for the agent's realm
 * (`createRun`) and for a run's hook realm (`createHooks`): one `Connection`, one `EventPipe`,
 * the attached `Browser`, the run's `BrowserContext` and the run page. Each realm evaluates its
 * own copy of the bundle (neo-runtime `quickjs.rs` `SDK_BUNDLE`), so the module state this sets
 * (the test id attribute, where reply events and listener errors go) is that realm's alone, and
 * the two realms never share an object.
 */

import { Browser } from './browser'
import { BrowserContext } from './browser-context'
import { Connection } from './connection'
import { EventPipe, onUncaughtListenerError } from './events'
import { setTestIdAttribute } from './locator'
import type { Page } from './page'
import { Playwright } from './playwright'
import { onReplyEvents } from './runtime-bridge'

/**
 * What neo-runtime hands `createRun` at run start (`quickjs.rs` `start`), and `createHooks`
 * inside `HooksInit`. Every value comes from a Rust constant or the run's tab, so the SDK derives
 * nothing on its own.
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

/** What {@link connectRun} builds for one realm. */
export interface RunObjects {
  connection: Connection
  /** `pipe.uncaught` rejects with the first listener error nobody handled in this realm. */
  pipe: EventPipe
  playwright: Playwright
  browser: Browser
  context: BrowserContext
  page: Page
}

/**
 * Builds a realm's objects around one `Connection` and one `EventPipe`. The run page is the
 * context's first page, so `context.pages()` returns this very object for it, and the context
 * is the first of `context.browser().contexts()`. Reads and sends nothing.
 */
export function connectRun(init: RunInit): RunObjects {
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
  playwright.selectors._attach(connection, init.context)
  const page = context._addRunPage(init.page, init.mainFrame, init.url)
  return { connection, pipe, playwright, browser, context, page }
}
