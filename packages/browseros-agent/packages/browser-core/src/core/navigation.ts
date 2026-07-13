import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { PageManager } from './pages'

// A browser navigation should not wait for every long-lived request on a
// modern app to finish. Facebook, Reddit, and similar sites keep analytics,
// streaming, and prefetch requests open long after the usable document exists.
// The agent can take a fresh snapshot and use explicit interaction waits when
// it needs a particular control.
const LOAD_TIMEOUT_MS = 8_000

const delay = (ms: number): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms))

/** Polls readyState until the document is usable (or times out). */
async function waitForLoad(
  session: ProtocolApi,
  timeout = LOAD_TIMEOUT_MS,
): Promise<void> {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    try {
      const result = await session.Runtime.evaluate({
        expression: 'document.readyState',
        returnByValue: true,
      })
      if (
        result.result?.value === 'interactive' ||
        result.result?.value === 'complete'
      ) return
    } catch {
      // Execution context torn down mid-navigation — expected; keep polling.
    }
    await delay(50)
  }
}

/** Navigation for a single page: url / reload / back / forward, each awaiting load. */
export class Navigation {
  constructor(
    private readonly pages: PageManager,
    private readonly pageId: number,
  ) {}

  async goto(url: string): Promise<void> {
    const { session } = await this.pages.getSession(this.pageId)
    await session.Page.navigate({ url })
    await waitForLoad(session)
  }

  async reload(): Promise<void> {
    const { session } = await this.pages.getSession(this.pageId)
    await session.Page.reload()
    await waitForLoad(session)
  }

  async back(): Promise<void> {
    await this.history('back')
  }

  async forward(): Promise<void> {
    await this.history('forward')
  }

  private async history(direction: 'back' | 'forward'): Promise<void> {
    const { session } = await this.pages.getSession(this.pageId)
    await session.Runtime.evaluate({
      expression: `history.${direction}()`,
      awaitPromise: true,
    })
    await waitForLoad(session)
  }
}
