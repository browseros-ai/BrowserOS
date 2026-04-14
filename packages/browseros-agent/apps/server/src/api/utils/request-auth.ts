import type { MiddlewareHandler } from 'hono'

const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]', '::1'])
const EXTENSION_PROTOCOLS = new Set(['chrome-extension:', 'moz-extension:'])

export function isTrustedAppOrigin(origin: string | undefined): boolean {
  if (!origin) return false

  try {
    const url = new URL(origin)

    if (
      (url.protocol === 'http:' || url.protocol === 'https:') &&
      LOOPBACK_HOSTS.has(url.hostname)
    ) {
      return true
    }

    return EXTENSION_PROTOCOLS.has(url.protocol)
  } catch {
    return false
  }
}

export function requireTrustedAppOrigin(): MiddlewareHandler {
  return async (c, next) => {
    const origin = c.req.header('origin')

    if (origin) {
      if (!isTrustedAppOrigin(origin)) {
        return c.json({ error: 'Forbidden' }, 403)
      }
      return next()
    }

    // Chrome extensions cannot set the Origin header (forbidden header).
    // When Origin is absent, allow if the request targets a loopback address
    // — the server only binds to loopback so only local processes can reach it.
    const host = c.req.header('host') ?? ''
    const hostname = host.split(':')[0]
    if (!LOOPBACK_HOSTS.has(hostname)) {
      return c.json({ error: 'Forbidden' }, 403)
    }

    return next()
  }
}
