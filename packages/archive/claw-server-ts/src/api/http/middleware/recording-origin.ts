import type { MiddlewareHandler } from 'hono'
import { canonicalApiError } from '../errors'
import { type RequestContextEnv, requestIdFor } from '../request-context'

/** Stable origin derived from claw-app's manifest signing key. */
const BROWSERCLAW_EXTENSION_ORIGIN =
  'chrome-extension://pjimfkbpehlcllblajnpfamdfjhhlgkc'

/** Blocks browser-page CSRF while preserving native contract clients. */
export const recordingIngestOriginHygiene: MiddlewareHandler<
  RequestContextEnv
> = async (c, next) => {
  const origin = c.req.header('origin')
  const trusted =
    origin === undefined ||
    origin === BROWSERCLAW_EXTENSION_ORIGIN ||
    (origin === 'null' && c.req.header('sec-fetch-site') === 'none')
  if (!trusted) {
    return c.json(
      canonicalApiError(
        'forbidden',
        'recording ingest is restricted to BrowserClaw',
        requestIdFor(c),
      ),
      403,
    )
  }
  return next()
}
