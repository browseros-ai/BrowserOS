/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Polls `GET /cockpit/tabs/activity` so the homepage can render a
 * live view of which tabs each agent has touched, how recently, and
 * what sequence of tool calls produced the current state. Backed by
 * the in-memory registry in
 * `apps/claw-server/src/lib/tab-activity/`; refer to that
 * module for the record shape, the active-window threshold, and the
 * RECENT_TOOLS_CAP that bounds `recentTools`.
 *
 * The route enriches each record with the agent profile (label,
 * harness, color), falling back to slug / null when the profile has
 * been deleted between record and read. The hook surfaces those
 * enriched fields directly so the screen does not have to join again.
 */

import { createQuery } from 'react-query-kit'
import { api } from './client'
import { parseResponse } from './parseResponse'

export interface ToolEvent {
  name: string
  at: number
}

export interface TabActivityRecord {
  targetId: string
  pageId: number
  url: string
  title: string
  agentId: string
  slug: string
  firstToolAt: number
  lastToolAt: number
  lastToolName: string
  toolCount: number
  recentTools: ToolEvent[]
  status: 'active' | 'idle'
  agentLabel: string
  harness: string | null
  color: string | null
}

interface TabsActivityResponse {
  tabs: TabActivityRecord[]
}

export const useTabsActivity = createQuery<TabsActivityResponse>({
  queryKey: ['tabs', 'activity'],
  fetcher: async () => {
    const res = await api.tabs.activity.$get()
    return parseResponse<TabsActivityResponse>(res)
  },
  refetchInterval: 1500,
})
