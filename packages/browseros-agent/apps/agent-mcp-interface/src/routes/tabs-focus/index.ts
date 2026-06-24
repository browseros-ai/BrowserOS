/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Powers the v2 homepage's Watch button. Looks up the agent's tab
 * group via the shared tracker, expands it, and activates the window
 * the group lives in so BrowserOS surfaces the live run.
 *
 * The work itself lives in `services/tab-group-ops.focusAgentTabGroup`;
 * this route is the HTTP shape.
 */

import { Hono } from 'hono'
import { getBrowserSession } from '../../lib/browser-session'
import { focusAgentTabGroup } from '../../services/tab-group-ops'

export const tabsFocusRoute = new Hono().post(
  '/tabs/focus/:agentId',
  async (c) => {
    const agentId = c.req.param('agentId')
    const session = getBrowserSession()
    if (!session) {
      return c.json({ ok: false, reason: 'browser session not connected' }, 503)
    }
    const result = await focusAgentTabGroup({ agentId, session })
    if (!result.ok) {
      return c.json(result, 404)
    }
    return c.json(result, 200)
  },
)
