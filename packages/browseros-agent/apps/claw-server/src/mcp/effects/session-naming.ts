/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { logger } from '../../lib/logger'
import type { ToolEffect } from '../dispatch'
import {
  maybeRequestSessionNaming,
  type SessionNamingServer,
} from '../session-naming'

/** Starts naming while the first successful tabs-new response stream is open. */
export function createSessionNamingEffect(
  server: SessionNamingServer,
): ToolEffect {
  let warnedMissingRequestId = false

  return ({ call, result }) => {
    if (result.isError || !call.flags.newPage) return undefined
    if (
      typeof call.requestId !== 'string' &&
      typeof call.requestId !== 'number'
    ) {
      if (!warnedMissingRequestId) {
        warnedMissingRequestId = true
        logger.warn('mcp session naming request id unavailable', {
          sessionId: call.sessionId,
        })
      }
      return undefined
    }

    void maybeRequestSessionNaming({
      server,
      sessionId: call.sessionId,
      requestId: call.requestId,
    }).catch((error) => {
      logger.warn('mcp session naming failed unexpectedly', {
        sessionId: call.sessionId,
        error: error instanceof Error ? error.message : String(error),
      })
    })
    return undefined
  }
}
