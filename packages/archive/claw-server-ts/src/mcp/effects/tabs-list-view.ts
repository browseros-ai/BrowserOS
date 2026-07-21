/**
 * @license
 * Copyright 2026 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { ownershipStore } from '../../domain/ownership'
import { identityService } from '../../lib/mcp-session'
import type { ToolEffect } from '../dispatch'
import {
  annotateTabsListWithOwnership,
  buildOwnershipDeps,
} from '../tabs-ownership'

/** Replaces successful tabs-list results with the caller's ownership view. */
export const applyTabsListView: ToolEffect = ({ call, result }) => {
  if (result.isError || !call.flags.listTabs || !call.identity) return
  const pages = (
    result.structuredContent as
      | { pages?: Array<{ page?: unknown }> }
      | undefined
  )?.pages
  const livePageIds = new Set<number>()
  for (const page of pages ?? []) {
    if (typeof page.page === 'number') livePageIds.add(page.page)
  }
  // Prune assumes the tabs list is the UNFILTERED full page set (it is
  // today — pages.list()); if listing ever grows filtering, prune must
  // move off this snapshot or it will drop live claims.
  ownershipStore.prune(livePageIds)
  const deps = buildOwnershipDeps(
    call.identity,
    ownershipStore,
    identityService,
  )
  return annotateTabsListWithOwnership(result, deps)
}
