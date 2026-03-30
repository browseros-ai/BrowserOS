/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Browser Service - Direct browser operations for SDK
 */

import type { Browser, PageInfo } from '../../../browser/browser'
import type {
  ActiveTab,
  InteractiveElements,
  NavigateResult,
  PageLoadStatus,
  Screenshot,
} from './types'
import { SdkError } from './types'

export class BrowserService {
  constructor(private browser: Browser) {}

  private selectPage(pages: PageInfo[], windowId?: number): PageInfo | null {
    const scopedPages =
      windowId === undefined
        ? pages
        : pages.filter((page) => page.windowId === windowId)
    if (scopedPages.length === 0) {
      return null
    }
    return (
      scopedPages.find((page) => page.isActive) ??
      scopedPages.find((page) => !page.isHidden) ??
      scopedPages[0]
    )
  }

  private async findExistingPage(windowId?: number): Promise<PageInfo | null> {
    if (windowId === undefined) {
      const activePage = await this.browser.getActivePage()
      if (activePage) {
        return activePage
      }
    }

    return this.selectPage(await this.browser.listPages(), windowId)
  }

  private async resolveExistingPage(windowId?: number): Promise<PageInfo> {
    const page = await this.findExistingPage(windowId)
    if (!page) {
      throw new SdkError(
        windowId === undefined
          ? 'No active tab found'
          : 'No tab found in specified window',
      )
    }
    return page
  }

  private async resolveNavigationPage(windowId?: number): Promise<PageInfo> {
    const existingPage = await this.findExistingPage(windowId)
    if (existingPage) {
      return existingPage
    }
    if (windowId !== undefined) {
      throw new SdkError('No tab found in specified window')
    }

    const pageId = await this.browser.newPage('about:blank', {
      background: false,
    })
    const createdPage = (await this.browser.listPages()).find(
      (page) => page.pageId === pageId,
    )
    if (!createdPage) {
      throw new SdkError('Failed to create a tab for navigation')
    }
    return createdPage
  }

  private async getPageIdForTab(tabId: number): Promise<number> {
    const resolved = await this.browser.resolveTabIds([tabId])
    const pageId = resolved.get(tabId)
    if (pageId === undefined) {
      throw new SdkError(`Tab ${tabId} not found`, 404)
    }
    return pageId
  }

  async getActiveTab(windowId?: number): Promise<ActiveTab> {
    const page = await this.resolveExistingPage(windowId)
    return {
      tabId: page.tabId,
      url: page.url,
      title: page.title,
      windowId: page.windowId ?? 0,
    }
  }

  async getPageContent(tabId: number): Promise<string> {
    const pageId = await this.getPageIdForTab(tabId)
    const content = await this.browser.contentAsMarkdown(pageId, {})
    if (!content) {
      throw new SdkError('No content found on page', 400)
    }
    return content
  }

  async getScreenshot(tabId: number): Promise<Screenshot> {
    const pageId = await this.getPageIdForTab(tabId)
    return await this.browser.screenshot(pageId, {
      format: 'png',
      fullPage: false,
    })
  }

  async navigate(
    url: string,
    tabId?: number,
    windowId?: number,
  ): Promise<NavigateResult> {
    if (tabId !== undefined) {
      const pages = await this.browser.listPages()
      const page = pages.find((p) => p.tabId === tabId)
      if (!page) {
        throw new SdkError(`Tab ${tabId} not found`, 404)
      }
      await this.browser.goto(page.pageId, url)
      return { tabId, windowId: page.windowId ?? 0 }
    }

    const activePage = await this.resolveNavigationPage(windowId)
    await this.browser.goto(activePage.pageId, url)
    return {
      tabId: activePage.tabId,
      windowId: activePage.windowId ?? 0,
    }
  }

  async getPageLoadStatus(tabId: number): Promise<PageLoadStatus> {
    const pages = await this.browser.listPages()
    const page = pages.find((p) => p.tabId === tabId)
    if (!page) {
      throw new SdkError('Tab not found', 404)
    }
    return {
      tabId: page.tabId,
      isDOMContentLoaded: !page.isLoading,
      isResourcesLoading: page.isLoading,
      isPageComplete: !page.isLoading,
    }
  }

  async getInteractiveElements(
    tabId: number,
    simplified = false,
    _windowId?: number,
  ): Promise<InteractiveElements> {
    const pageId = await this.getPageIdForTab(tabId)
    const content = simplified
      ? await this.browser.snapshot(pageId)
      : await this.browser.enhancedSnapshot(pageId)
    return { content }
  }
}
