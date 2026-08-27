/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { MiddlewareHandler } from 'hono'

// Browsers always send a Sec-Fetch-Site header; native MCP clients and the
// internal ACP client never do. Rejecting it blocks browser-originated requests
// (DNS rebinding, CSRF) against the LAN-exposed MCP endpoint without restricting
// the bind address. Mirrors the claw-server's request hygiene.
export function rejectBrowserFetch(): MiddlewareHandler {
  return async (c, next) => {
    if (c.req.header('Sec-Fetch-Site') !== undefined) {
      return c.json(
        {
          error: {
            name: 'ForbiddenBrowserRequest',
            message: 'Browser requests are not allowed on this endpoint',
            code: 'FORBIDDEN_BROWSER_REQUEST',
            statusCode: 403,
          },
        },
        403,
      )
    }
    return next()
  }
}
