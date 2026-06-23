/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Polls `GET /cockpit/tabs/activity` so the homepage can render a
 * live view of which tabs each agent has touched and how recently.
 * Backed by the in-memory registry in
 * `apps/agent-mcp-interface/src/lib/tab-activity/`; refer to that
 * module for the record shape and the active-window threshold.
 */

import { createQuery } from 'react-query-kit'
import { api } from './client'
import { parseResponse } from './parseResponse'

export interface TabActivityRecord {
  targetId: string
  pageId: number
  url: string
  title: string
  agentId: string
  slug: string
  lastToolAt: number
  lastToolName: string
  status: 'active' | 'idle'
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
