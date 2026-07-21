/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { extractPageId, tabActivityRegistry } from '../../lib/tab-activity'
import type { ToolEffect } from '../dispatch'

/** Records successful page-targeted dispatches in the activity registry. */
export const applyTabActivity: ToolEffect = ({ call, result }) => {
  if (result.isError || !call.agent || !call.session) return undefined
  const pageId = extractPageId(call.tool.name, call.args)
  if (pageId === null) return undefined
  const live = call.session.pages.getInfo(pageId)
  if (!live) return undefined
  tabActivityRegistry.recordTool({
    sessionId: call.sessionId,
    agentId: call.agent.agentId,
    slug: call.agent.slug,
    tabId: live.tabId,
    pageId,
    targetId: live.targetId,
    toolName: call.tool.name,
  })
  return undefined
}
