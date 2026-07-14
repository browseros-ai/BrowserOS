/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Read endpoint backing the cockpit homepage's "which tabs are
 * being driven right now" view. The registry behind this route is
 * fed by `apps/claw-server/src/mcp/register.ts` every time a
 * browser tool dispatch succeeds; this route just publishes the
 * current snapshot.
 *
 * The snapshot is joined against the live MCP identity registry so
 * the UI receives the connecting client's label. Records outlive a
 * closed MCP session until the activity TTL expires, so missing
 * identities fall back to the stable recorded slug.
 *
 * Polling is the v1 transport (the UI hook polls every 1500 ms); SSE
 * on `?stream=1` is a future option if polling proves chatty.
 */

import { Hono } from 'hono'
import { agentIdentityFromClient, identityService } from '../../lib/mcp-session'
import {
  type TabActivityRecord,
  tabActivityRegistry,
} from '../../lib/tab-activity'
import { screencastCache } from '../../services/screencast-cache'
import { resolveAgentDisplay } from './agent-display'

interface EnrichedTabRecord extends TabActivityRecord {
  agentLabel: string
  harness: string | null
  color: string | null
  /**
   * Latest screencast frame from the background poller. `null` when
   * the cache has no frame for the pageId (poller has not yet run,
   * page is in failure backoff, or the tab is idle).
   */
  screencast: { jpegBase64: string; capturedAt: number } | null
}

export const tabsRoute = new Hono().get('/tabs/activity', (c) => {
  const tabs = tabActivityRegistry.snapshot()
  if (tabs.length === 0) {
    return c.json({ tabs: [] as EnrichedTabRecord[] })
  }
  // O(records + identities) in-memory join. The resolver picks the
  // live session identity when present, then the recorded slug.
  const identitiesByAgentId = new Map<
    string,
    ReturnType<typeof identityService.list>[number]
  >()
  for (const identity of identityService.list()) {
    const { agentId } = agentIdentityFromClient(identity)
    if (!identitiesByAgentId.has(agentId)) {
      identitiesByAgentId.set(agentId, identity)
    }
  }
  const enriched: EnrichedTabRecord[] = tabs.map((tab) => {
    const display = resolveAgentDisplay(
      tab.agentId,
      tab.slug,
      identitiesByAgentId,
    )
    const frame = screencastCache.get(tab.pageId)
    const screencast = frame
      ? { jpegBase64: frame.jpegBase64, capturedAt: frame.capturedAt }
      : null
    return { ...tab, ...display, screencast }
  })
  return c.json({ tabs: enriched })
})
