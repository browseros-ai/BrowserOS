import { BrowserContext } from './browser-context'
import { Connection } from './connection'
import { EventPipe, onUncaughtListenerError } from './events'
import { setTestIdAttribute } from './locator'
import type { Page } from './page'
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
 * Builds the `page` and `context` a run's script receives, around one `Connection` and one
 * `EventPipe`. The run page is the context's first page, so `context.pages()` returns this very
 * object for it. `uncaught` rejects with the first listener error nobody handled; neo-runtime
 * races it with the script, so such an error ends the run as it would end a Node process.
 */
export function createRun(init: RunInit): {
  page: Page
  context: BrowserContext
  uncaught: Promise<never>
} {
  setTestIdAttribute(init.testIdAttribute)
  const connection = new Connection(init.lineOffset)
  const pipe = new EventPipe()
  onReplyEvents((until) => pipe.deliver(until))
  onUncaughtListenerError((error) => pipe.report(error))
  const context = new BrowserContext(init.context, connection, pipe)
  const page = context._addRunPage(init.page, init.mainFrame, init.url)
  return { page, context, uncaught: pipe.uncaught }
}
