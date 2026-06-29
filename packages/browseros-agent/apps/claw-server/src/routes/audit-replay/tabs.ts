/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * `GET /replay/tabs` — the cockpit's "which BrowserOS tabs are
 * currently agent-driven and therefore should be recorded" surface.
 *
 * Mounted at the root path `/replay/tabs` (not under `/audit/replay`
 * like the event POST/GET) because this route is consumed by the
 * extension background worker, not by the audit UI. Keeping the
 * sub-tree shallow makes the extension's polling URL easy to read
 * + reason about, separate from the audit-replay event surface.
 *
 * Auth: unauthenticated, consistent with the rest of the cockpit
 * loopback surface. The response only enumerates the tabs an agent
 * is currently driving; a malicious local page could in theory
 * deduce some session context from it, but the same risk applies
 * to /tabs/activity (already public) and the rest of the audit
 * surface. Whole-loopback hardening is a separate concern.
 */

import { Hono } from 'hono'
import { replayTabsService } from '../../services/replay-tabs'

export const replayTabsRoute = new Hono().get('/replay/tabs', (c) => {
  return c.json({ tabs: replayTabsService.list() })
})
