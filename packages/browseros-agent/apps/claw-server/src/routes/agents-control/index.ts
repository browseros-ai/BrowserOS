/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Powers the homepage's Stop button. Aborts every in-flight tool
 * dispatch for the selected agent record and returns the count so
 * the UI can surface it. The session itself stays open; the agent's
 * harness sees the cancelled dispatch as an isError tool result and
 * is free to fire its next call.
 */

import { Hono } from 'hono'
import { logger } from '../../lib/logger'
import {
  CANCELLATION_REASON,
  dispatchCancellation,
} from '../../services/dispatch-cancellation'

export const agentsControlRoute = new Hono().post(
  '/agents/:agentId/cancel',
  (c) => {
    const agentId = c.req.param('agentId')
    const cancelled = dispatchCancellation.cancelByAgent(
      agentId,
      CANCELLATION_REASON,
    )
    if (cancelled === 0) {
      return c.json(
        {
          ok: false,
          cancelled: 0,
          reason: 'no active dispatches for this agent',
        },
        404,
      )
    }
    // The operator's Stop button: pairs with the cancelled-dispatch
    // audit rows so a mid-run abort is attributable in the log.
    logger.info('cancelled in-flight dispatches for agent', {
      agentId,
      cancelled,
    })
    return c.json({ ok: true, cancelled }, 200)
  },
)
