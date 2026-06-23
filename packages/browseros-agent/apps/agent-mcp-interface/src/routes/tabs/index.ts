/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Read endpoint backing the cockpit homepage's "which tabs are
 * being driven right now" view. The registry behind this route is
 * fed by `apps/agent-mcp-interface/src/mcp/register.ts` every time a
 * browser tool dispatch succeeds; this route just publishes the
 * current snapshot. Polling is the v1 transport (the UI hook polls
 * every 1500 ms); SSE on `?stream=1` is a future option if polling
 * proves chatty.
 */

import { Hono } from 'hono'
import { tabActivityRegistry } from '../../lib/tab-activity'

export const tabsRoute = new Hono().get('/tabs/activity', (c) =>
  c.json({ tabs: tabActivityRegistry.snapshot() }),
)
