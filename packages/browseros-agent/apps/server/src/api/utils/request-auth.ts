import type { MiddlewareHandler } from 'hono'

const LOOPBACK_HOSTS = new Set(['127.0.0.1', 'localhost', '[::1]', '::1'])
const EXTENSION_PROTOCOLS = new Set(['chrome-extension:', 'moz-extension:'])

function isLoopbackRequestTarget(urlString: string): boolean {
  try {
    const url = new URL(urlString)
    return LOOPBACK_HOSTS.has(url.hostname)
  } catch {
    return false
  }
}

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

    // Browser extension fetches for simple read-only endpoints may omit Origin.
    // Allow origin-less loopback reads, but keep mutating routes origin-gated.
    if (
      !origin &&
      ['GET', 'HEAD', 'OPTIONS'].includes(c.req.method) &&
      isLoopbackRequestTarget(c.req.url)
    ) {
      return next()
    }

    if (!isTrustedAppOrigin(origin)) {
      return c.json({ error: 'Forbidden' }, 403)
    }

    return next()
  }
}
