/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { MiddlewareHandler } from 'hono'
import { isAllowedOrigin } from '../utils/cors'

/**
 * Reject requests whose `Origin` header is present and not in
 * the trusted-origins allowlist.
 *
 * Hono's `cors()` middleware on its own only omits the
 * response `Access-Control-Allow-Origin` header on a
 * mismatch — by the time the browser blocks the read, the
 * route handler has already executed. Routes that have side
 * effects (running a tool, mutating state) need an active
 * gate, not a response-side one. This middleware provides
 * that.
 *
 * Permissive on missing `Origin`. The header is only present
 * on browser-initiated cross-origin requests. CLI tools,
 * internal Node clients, and some service-worker fetches
 * legitimately omit it; rejecting those would break working
 * integrations. Browser-initiated POST requests with bodies
 * always carry `Origin` (the header is on the Forbidden
 * Header List, so JS cannot suppress it), so the threat
 * model is fully covered.
 */
export function requireTrustedOrigin(): MiddlewareHandler {
  return async (c, next) => {
    const origin = c.req.header('Origin')
    if (origin !== undefined && !isAllowedOrigin(origin)) {
      return c.json(
        {
          error: {
            name: 'ForbiddenOrigin',
            message: 'Origin not allowed',
            code: 'FORBIDDEN_ORIGIN',
            statusCode: 403,
          },
        },
        403,
      )
    }
    return next()
  }
}
