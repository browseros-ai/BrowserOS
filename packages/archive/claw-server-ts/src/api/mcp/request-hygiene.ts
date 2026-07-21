/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { MiddlewareHandler } from 'hono'

const WRITE_METHODS = new Set(['POST', 'PUT', 'PATCH', 'DELETE'])

/** Enforces the header conventions native MCP clients follow. */
export const setMcpRequestHygieneMiddleware: MiddlewareHandler = async (
  c,
  next,
) => {
  const headers = c.req.raw.headers
  if (headers.has('origin') || headers.has('sec-fetch-site')) {
    return c.json({ error: 'unsupported request' }, 403)
  }
  if (WRITE_METHODS.has(c.req.method)) {
    const ct = (headers.get('content-type') ?? '').toLowerCase()
    if (!ct.includes('application/json')) {
      return c.json({ error: 'unsupported content type' }, 415)
    }
  }
  await next()
  return
}
