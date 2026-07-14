/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ownershipStore } from '../../domain/ownership'
import { tabActivityRegistry } from '../../lib/tab-activity'
import type { ToolEffect } from '../dispatch'

/** Updates ownership for successful tab creation and closure results. */
export const applyOwnershipClaims: ToolEffect = ({ call, result }) => {
  if (result.isError || !call.agent || !call.key) return undefined

  if (call.flags.newPage) {
    const pageId = (result.structuredContent as { page?: number } | undefined)
      ?.page
    if (typeof pageId !== 'number') return undefined

    // `tabs new` has no page in its args; the page id is born in the result.
    const live = call.session?.pages.getInfo(pageId)
    if (live) {
      tabActivityRegistry.recordTool({
        agentId: call.agent.agentId,
        slug: call.agent.slug,
        pageId,
        targetId: live.targetId,
        toolName: 'tabs',
      })
    }
    // The isolation ledger grants this agent access to the result-born page.
    ownershipStore.claimPage(call.key, pageId)
    return undefined
  }

  if (!call.flags.closePage) return undefined
  const page = (call.args as { page?: unknown } | null)?.page
  if (typeof page === 'number' && Number.isInteger(page) && page >= 1) {
    ownershipStore.releasePage(call.key, page)
  }
  return undefined
}
