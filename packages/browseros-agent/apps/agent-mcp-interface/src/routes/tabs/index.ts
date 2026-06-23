/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Read endpoint backing the cockpit homepage's "which tabs are
 * being driven right now" view. The registry behind this route is
 * fed by `apps/agent-mcp-interface/src/mcp/register.ts` every time a
 * browser tool dispatch succeeds; this route just publishes the
 * current snapshot.
 *
 * The snapshot is joined against the agents directory so the UI
 * receives `agentLabel` and `harness` directly instead of a slug it
 * has to format itself. Profile lookups fall back to the slug when a
 * record references an agent whose stored profile has been deleted;
 * the route never throws on a missing profile.
 *
 * Polling is the v1 transport (the UI hook polls every 1500 ms); SSE
 * on `?stream=1` is a future option if polling proves chatty.
 */

import { Hono } from 'hono'
import {
  type TabActivityRecord,
  tabActivityRegistry,
} from '../../lib/tab-activity'
import { list as listAgents } from '../agents/service'

export interface EnrichedTabRecord extends TabActivityRecord {
  agentLabel: string
  harness: string | null
  // No stored colour on the agent profile yet; emit null so the UI
  // falls back to its slug-hash palette. Wire is ready for the day
  // the profile schema gains a `color` field.
  color: string | null
}

export const tabsRoute = new Hono().get('/tabs/activity', async (c) => {
  const tabs = tabActivityRegistry.snapshot()
  if (tabs.length === 0) {
    return c.json({ tabs: [] as EnrichedTabRecord[] })
  }
  // O(records + profiles) join. The agents directory reads from disk
  // on every call today; if the read becomes a bottleneck we can add
  // a stale-while-revalidate cache next to the registry.
  const profiles = await listAgents()
  const byId = new Map(profiles.map((p) => [p.id, p]))
  const enriched: EnrichedTabRecord[] = tabs.map((tab) => {
    const profile = byId.get(tab.agentId)
    return {
      ...tab,
      agentLabel: profile?.name ?? tab.slug,
      harness: profile?.harness ?? null,
      color: null,
    }
  })
  return c.json({ tabs: enriched })
})
