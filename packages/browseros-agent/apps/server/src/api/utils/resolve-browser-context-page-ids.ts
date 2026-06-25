/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type { Browser } from '@browseros/browser-core/browser'
import type { BrowserContext } from '@browseros/shared/schemas/browser-context'
import { logger } from '../../lib/logger'

export async function resolveBrowserContextPageIds(
  browser: Pick<Browser, 'resolveTabIds'>,
  browserContext?: BrowserContext,
): Promise<BrowserContext | undefined> {
  if (!browserContext) return undefined

  const tabIdSet = new Set<number>()
  if (browserContext.activeTab) tabIdSet.add(browserContext.activeTab.id)
  if (browserContext.selectedTabs) {
    for (const tab of browserContext.selectedTabs) tabIdSet.add(tab.id)
  }
  if (browserContext.tabs) {
    for (const tab of browserContext.tabs) tabIdSet.add(tab.id)
  }

  if (tabIdSet.size === 0) return browserContext

  const tabToPage = await browser.resolveTabIds([...tabIdSet])

  const addPageId = (tab: { id: number; url?: string; title?: string }) => {
    const pageId = tabToPage.get(tab.id)
    if (pageId === undefined) {
      logger.warn('Could not resolve page ID for tab', { tabId: tab.id })
    }
    return { ...tab, pageId }
  }

  logger.debug('Resolved tab IDs to page IDs', {
    mapping: Object.fromEntries(tabToPage),
  })

  return {
    ...browserContext,
    activeTab: browserContext.activeTab
      ? addPageId(browserContext.activeTab)
      : undefined,
    selectedTabs: browserContext.selectedTabs?.map(addPageId),
    tabs: browserContext.tabs?.map(addPageId),
  }
}
