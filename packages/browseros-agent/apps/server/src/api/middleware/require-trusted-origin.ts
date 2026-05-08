/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { MiddlewareHandler } from 'hono'
import { isAllowedOrigin } from '../utils/cors'

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
