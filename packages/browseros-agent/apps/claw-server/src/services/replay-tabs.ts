/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Builds the read shape behind `GET /replay/tabs`. The cockpit's
 * extension background worker polls that endpoint every ~2s to
 * discover which BrowserOS tabs are currently agent-driven, then
 * uses `chrome.tabs.query({url})` + a `groupColor` discriminator
 * (this file emits the colour) to resolve each row to a chrome
 * tab id and inject the recorder via `chrome.scripting.executeScript`.
 *
 * Inputs:
 *   - tabActivityRegistry.snapshot() for the live (CDP-side) tabs
 *     each agent is driving. Each record carries `agentId`,
 *     `pageId`, `targetId`, `url`, `title`.
 *   - identityService.list() to map agentId back to the most
 *     recent live MCP `sessionId` (the replay file name).
 *   - tabGroupTracker.list() to map agentId back to the agent's
 *     tab-group colour, used by the extension to disambiguate
 *     when two agents happen to open the same URL.
 *
 * Output:
 *   - `[{ sessionId, tabPageId, url, title, groupColor }]`.
 *
 * Sessions without a matching live identity are dropped from the
 * response: if no MCP session is currently registered for this
 * agentId, there is nothing to POST events to, so the extension
 * has nothing to record. The next /replay/tabs poll picks the
 * tab up again as soon as a session reattaches.
 *
 * Multi-session edge case: if two MCP sessions share a clientName
 * (and therefore an agentId), the lookup returns one of them
 * arbitrarily. Each agent has its own tab group so the tab in
 * question belongs to ONE of the sessions in practice; v1 accepts
 * the rare misattribution. A future iteration could plumb the
 * dispatching sessionId through to tabActivityRegistry.recordTool
 * so the registry tracks it directly.
 */

import {
  type TabGroupColor,
  type TabGroupTracker,
  tabGroupTracker,
} from '../lib/agent-tab-groups'
import {
  agentIdentityFromClient,
  type ClientIdentity,
  type IdentityService,
  identityService,
} from '../lib/mcp-session'
import {
  type TabActivityRegistry,
  tabActivityRegistry,
} from '../lib/tab-activity'

export interface ReplayTab {
  sessionId: string
  tabPageId: number
  url: string
  title: string
  /**
   * Chrome tab-group colour for the agent that owns this tab. The
   * extension background worker uses this to disambiguate when two
   * agents have the same URL open in two different tab groups.
   * `null` when no group has been registered yet (rare race; the
   * tab group is created right after the agent's first `tabs new`).
   */
  groupColor: TabGroupColor | null
}

export interface ReplayTabsServiceDeps {
  registry: Pick<TabActivityRegistry, 'snapshot'>
  identityService: Pick<IdentityService, 'list'>
  tabGroupTracker: Pick<TabGroupTracker, 'getByAgentId'>
}

export function createReplayTabsService(deps: ReplayTabsServiceDeps) {
  return {
    list(): ReplayTab[] {
      const liveByAgentId = buildLiveAgentIdMap(deps.identityService.list())
      const tabs = deps.registry.snapshot()
      const out: ReplayTab[] = []
      for (const tab of tabs) {
        const identity = liveByAgentId.get(tab.agentId)
        if (!identity) continue
        const group = deps.tabGroupTracker.getByAgentId(tab.agentId)
        out.push({
          sessionId: identity.sessionId,
          tabPageId: tab.pageId,
          url: tab.url,
          title: tab.title,
          groupColor: group?.color ?? null,
        })
      }
      return out
    },
  }
}

function buildLiveAgentIdMap(
  identities: ReadonlyArray<ClientIdentity>,
): Map<string, ClientIdentity> {
  const map = new Map<string, ClientIdentity>()
  for (const identity of identities) {
    const { agentId } = agentIdentityFromClient(identity)
    if (!map.has(agentId)) {
      map.set(agentId, identity)
    }
  }
  return map
}

/** Process-wide singleton consumed by the route. */
export const replayTabsService = createReplayTabsService({
  registry: tabActivityRegistry,
  identityService,
  tabGroupTracker,
})
