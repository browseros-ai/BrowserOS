/** Neo: Host capabilities outside Playwright's object model. The namespace is always present;
 * native admission decides whether this host offers a solver. No keys cross this boundary. */
import type { Connection } from './connection'
import type { NeoCaptchaOutcome } from './neo-commands'
import { Page } from './page'

/** The runtime owns cancellation and page input until the host's solver and cleanup settle. */
export function createNeo(connection: Connection): Readonly<{
  solveCaptcha(page: Page): Promise<NeoCaptchaOutcome>
}> {
  return Object.freeze({
    solveCaptcha(page: Page): Promise<NeoCaptchaOutcome> {
      if (!(page instanceof Page))
        throw new TypeError('neo.solveCaptcha takes a page')
      return connection.sendNeo(
        'neo.solveCaptcha',
        page.mainFrame()._guid,
        'neo:solveCaptcha',
        {},
        0,
      )
    },
  })
}
