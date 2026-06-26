/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * react-query-kit mutation backing the homepage's Watch button.
 *
 * Drives the BrowserOS Chrome tab activation entirely via the
 * extension's `chrome.tabs.*` and `chrome.windows.*` APIs. The
 * earlier server route (`POST /tabs/focus/:agentId`) only expanded
 * the agent's tab group, which is not what the operator wants when
 * they click Watch: they want to actually switch BrowserOS to the
 * tab the agent is currently working on. Doing that requires a
 * chrome tab id, which the extension already has via
 * `chrome.tabs.query`. The server has no business in the loop.
 *
 * Match strategy: query by the focus tab's URL. If multiple tabs
 * share the URL we take the first match (rare in cockpit usage; the
 * cockpit's per-agent tab groups isolate sessions). If no tab
 * matches (e.g. the tab was closed since the registry last saw it)
 * we return `ok: false` with a reason and the call site logs to the
 * console. No toast surface in v2 yet.
 */

import { createMutation } from 'react-query-kit'

export interface FocusAgentResult {
  ok: boolean
  tabId?: number
  windowId?: number
  reason?: string
}

interface FocusAgentVariables {
  agentId: string
  /**
   * URL of the agent's freshest tab, sourced from
   * `agent.currentFocus.url`. Used as the chrome.tabs.query filter.
   */
  focusUrl: string
}

export const useFocusAgent = createMutation<
  FocusAgentResult,
  FocusAgentVariables
>({
  mutationFn: async ({ focusUrl }) => {
    // chrome.tabs accepts a match pattern in `url`. The agent's
    // focus URL is an exact string and matches itself.
    const matches = await chrome.tabs.query({ url: focusUrl })
    const tab = matches[0]
    if (!tab || typeof tab.id !== 'number') {
      return {
        ok: false,
        reason: `no chrome tab matches ${focusUrl}`,
      }
    }
    await chrome.tabs.update(tab.id, { active: true })
    if (typeof tab.windowId === 'number') {
      await chrome.windows.update(tab.windowId, { focused: true })
    }
    return {
      ok: true,
      tabId: tab.id,
      windowId: tab.windowId,
    }
  },
})
