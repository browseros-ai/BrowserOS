import type { MiddlewareHandler } from 'hono'
import { isLocalhostRequest } from './security'

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

    // Some local reads arrive without an Origin header. Allow those only when
    // the actual client socket is loopback. This avoids Host-header spoofing.
    if (
      ['GET', 'HEAD', 'OPTIONS'].includes(c.req.method) &&
      isLocalhostRequest(c)
    ) {
      return next()
    }

    return c.json({ error: 'Forbidden' }, 403)
  }
}
