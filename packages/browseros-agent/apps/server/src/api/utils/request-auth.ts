import type { Context, MiddlewareHandler } from 'hono'
import type { Env } from '../types'
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
    if (isTrustedAppRequest(c)) {
      return next()
    }

    return c.json({ error: 'Forbidden' }, 403)
  }
}

export function isTrustedAppRequest(c: Context<Env>): boolean {
  if (!isLocalhostRequest(c)) return false

  const origin = c.req.header('origin')
  if (origin) return isTrustedAppOrigin(origin)

  return ['GET', 'HEAD', 'OPTIONS'].includes(c.req.method)
}
