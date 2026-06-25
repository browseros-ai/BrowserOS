import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import { logger } from '../logger'
import {
  type CdpConnection,
  EXCLUDED_URL_PREFIXES,
  type SessionId,
} from './connection'

export interface PageInfo {
  pageId: number
  targetId: string
  tabId: number
  url: string
  title: string
  isActive: boolean
  isLoading: boolean
  loadProgress: number
  isPinned: boolean
  isHidden: boolean
  windowId?: number
  index?: number
  groupId?: string
}

// Shape returned by the custom Browser.* CDP domain (a PageInfo without our synthetic pageId).
type TabInfo = Omit<PageInfo, 'pageId'>
type WindowInfo = {
  windowId: number
  isVisible: boolean
  isActive: boolean
}

export interface PageSession {
  targetId: string
  session: ProtocolApi
  url: string
}

export interface PageManagerHooks {
  onSessionAttached?: (
    session: ProtocolApi,
    pageId: number,
    sessionId: string,
  ) => Promise<void>
  onPageDetached?: (pageId: number) => void
}

const delay = (ms: number): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms))

/** Owns the stable pageId registry and its attached CDP tab sessions. */
export class PageManager {
  private readonly pages = new Map<number, PageInfo>()
  private readonly sessions = new Map<string, SessionId>()
  private connectionEpoch: number
  private nextPageId = 1
  private hiddenWindowId?: number

  constructor(
    private readonly cdp: CdpConnection,
    private readonly hooks: PageManagerHooks = {},
  ) {
    this.connectionEpoch = cdp.connectionEpoch()
  }

  /** Reconcile the registry with the browser's live tabs (upsert + drop vanished). */
  async list(): Promise<PageInfo[]> {
    await this.ensureConnected()
    const result = await this.cdp.Browser.getTabs({ includeHidden: true })
    const tabs = (result.tabs as TabInfo[]).filter(
      (tab) =>
        !EXCLUDED_URL_PREFIXES.some((prefix) => tab.url.startsWith(prefix)),
    )

    const seen = new Set<string>()
    for (const tab of tabs) {
      seen.add(tab.targetId)
      const existing =
        this.findByTarget(tab.targetId) ?? this.findByTab(tab.tabId)
      if (existing) {
        if (existing.targetId !== tab.targetId) {
          this.sessions.delete(existing.targetId)
        }
        // CDP omits windowId for hidden tabs — preserve the cached value.
        Object.assign(existing, tab, {
          windowId: tab.windowId ?? existing.windowId,
        })
      } else {
        const pageId = this.nextPageId++
        this.pages.set(pageId, { pageId, ...tab })
      }
    }

    for (const [pageId, info] of this.pages) {
      if (!seen.has(info.targetId)) {
        this.pages.delete(pageId)
        this.sessions.delete(info.targetId)
        this.hooks.onPageDetached?.(pageId)
      }
    }

    return [...this.pages.values()].sort((a, b) => a.pageId - b.pageId)
  }

  getInfo(pageId: number): PageInfo | undefined {
    return this.pages.get(pageId)
  }

  getTabId(pageId: number): number | undefined {
    return this.pages.get(pageId)?.tabId
  }

  /** Resolve a pageId to its attached CDP session, listing pages first if unseen. */
  async getSession(pageId: number): Promise<PageSession> {
    const reconnected = await this.ensureConnected()
    let info = this.pages.get(pageId)
    if (!info || reconnected) {
      await this.list()
      info = this.pages.get(pageId)
    }
    if (!info) {
      throw new Error(`Unknown page ${pageId}. List pages to see what is open.`)
    }
    const sessionId = await this.attach(info.targetId, pageId)
    return {
      targetId: info.targetId,
      session: this.cdp.session(sessionId),
      url: info.url,
    }
  }

  getAttachedSession(pageId: number): ProtocolApi | null {
    const info = this.pages.get(pageId)
    if (!info) return null
    const sessionId = this.sessions.get(info.targetId)
    return sessionId ? this.cdp.session(sessionId) : null
  }

  async getActive(): Promise<PageInfo | null> {
    await this.ensureConnected()
    const result = await this.cdp.Browser.getActiveTab()
    if (!result.tab) return null

    await this.list()
    const tab = result.tab as TabInfo
    return this.findByTarget(tab.targetId) ?? null
  }

  async getActiveSessionForWindow(windowId: number): Promise<PageSession> {
    await this.ensureConnected()
    const result = await this.cdp.Browser.getActiveTab({ windowId })
    const tab = result.tab as TabInfo | undefined
    if (!tab) throw new Error(`No active tab in window ${windowId}`)

    const pageId = await this.ensurePageIdForTarget(tab.targetId)
    const sessionId = await this.attach(tab.targetId, pageId)
    return {
      targetId: tab.targetId,
      session: this.cdp.session(sessionId),
      url: tab.url,
    }
  }

  async refresh(pageId: number): Promise<PageInfo | undefined> {
    await this.ensureConnected()
    let info = this.pages.get(pageId)
    if (!info) {
      await this.list()
      info = this.pages.get(pageId)
    }
    if (!info) return undefined

    try {
      const result = await this.cdp.Browser.getTabInfo({ tabId: info.tabId })
      const tab = result.tab as TabInfo
      const updated: PageInfo = {
        ...info,
        ...tab,
        windowId: tab.windowId ?? info.windowId,
      }
      this.pages.set(pageId, updated)
      return updated
    } catch {
      await this.list()
      return this.pages.get(pageId)
    }
  }

  async resolveTabIds(tabIds: number[]): Promise<Map<number, number>> {
    await this.list()
    const tabToPage = new Map<number, number>()
    for (const info of this.pages.values()) {
      if (tabIds.includes(info.tabId)) tabToPage.set(info.tabId, info.pageId)
    }
    return tabToPage
  }

  async newPage(
    url: string,
    opts?: {
      background?: boolean
      hidden?: boolean
      windowId?: number
      tabGroupId?: string
    },
  ): Promise<number> {
    await this.ensureConnected()
    const windowId = await this.resolveWindowIdForNewPage(opts)
    const created = await this.cdp.Browser.createTab({
      url,
      ...(opts?.background !== undefined && { background: opts.background }),
      ...(windowId !== undefined && { windowId }),
    })
    const tabId = (created.tab as TabInfo).tabId

    let tab: TabInfo | undefined
    for (let attempt = 0; attempt < 30; attempt++) {
      try {
        tab = (await this.cdp.Browser.getTabInfo({ tabId })).tab as TabInfo
        if (!tab.isLoading || tab.loadProgress >= 1) break
      } catch {}
      await delay(100)
    }
    if (!tab) throw new Error(`Tab ${tabId} not found after creation`)

    if (opts?.tabGroupId) {
      try {
        await this.cdp.Browser.addTabsToGroup({
          groupId: opts.tabGroupId,
          tabIds: [tabId],
        })
        tab = (await this.cdp.Browser.getTabInfo({ tabId })).tab as TabInfo
      } catch (error) {
        logger.warn('Failed to add new page to default tab group', {
          tabGroupId: opts.tabGroupId,
          tabId,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }

    const pageId = this.nextPageId++
    this.pages.set(pageId, { pageId, ...tab, url: tab.url || url })
    return pageId
  }

  private async resolveWindowIdForNewPage(opts?: {
    hidden?: boolean
    windowId?: number
  }): Promise<number | undefined> {
    if (!opts?.hidden) {
      if (opts?.windowId !== undefined) return opts.windowId
      return undefined
    }

    const windows = (await this.cdp.Browser.getWindows())
      .windows as WindowInfo[]
    if (opts.windowId !== undefined) {
      const targetWindow = windows.find(
        (window) => window.windowId === opts.windowId,
      )
      if (targetWindow && !targetWindow.isVisible) {
        this.hiddenWindowId = targetWindow.windowId
        return targetWindow.windowId
      }
      if (targetWindow?.isVisible) {
        logger.warn(
          'Requested hidden page target window is visible, creating a new hidden window instead',
          { requestedWindowId: opts.windowId },
        )
      }
      const hiddenWindow = await this.cdp.Browser.createWindow({ hidden: true })
      this.hiddenWindowId = (hiddenWindow.window as WindowInfo).windowId
      return this.hiddenWindowId
    }

    if (this.hiddenWindowId !== undefined) {
      const cachedWindow = windows.find(
        (window) => window.windowId === this.hiddenWindowId,
      )
      if (cachedWindow && !cachedWindow.isVisible) return cachedWindow.windowId
      this.hiddenWindowId = undefined
    }

    const hiddenWindow = await this.cdp.Browser.createWindow({ hidden: true })
    this.hiddenWindowId = (hiddenWindow.window as WindowInfo).windowId
    return this.hiddenWindowId
  }

  async close(pageId: number): Promise<void> {
    const info = this.pages.get(pageId)
    if (!info) throw new Error(`Unknown page ${pageId}.`)
    await this.cdp.Browser.closeTab({ tabId: info.tabId })
    this.pages.delete(pageId)
    this.sessions.delete(info.targetId)
    this.hooks.onPageDetached?.(pageId)
  }

  async show(
    pageId: number,
    opts?: { windowId?: number; index?: number; activate?: boolean },
  ): Promise<PageInfo> {
    await this.ensureConnected()
    const info = (await this.refresh(pageId)) ?? this.requireInfo(pageId)
    if (!info.isHidden) {
      throw new Error(`Page ${pageId} is already visible.`)
    }

    const result = await this.cdp.Browser.showTab({
      tabId: info.tabId,
      ...(opts?.windowId !== undefined && { windowId: opts.windowId }),
      ...(opts?.index !== undefined && { index: opts.index }),
      ...(opts?.activate !== undefined && { activate: opts.activate }),
    })
    return this.updateFromTab(pageId, result.tab as TabInfo)
  }

  async move(
    pageId: number,
    opts?: { windowId?: number; index?: number },
  ): Promise<PageInfo> {
    await this.ensureConnected()
    const info = (await this.refresh(pageId)) ?? this.requireInfo(pageId)
    const result = await this.cdp.Browser.moveTab({
      tabId: info.tabId,
      ...(opts?.windowId !== undefined && { windowId: opts.windowId }),
      ...(opts?.index !== undefined && { index: opts.index }),
    })
    return this.updateFromTab(pageId, result.tab as TabInfo)
  }

  detachSession(sessionId: SessionId): void {
    for (const [targetId, sid] of this.sessions) {
      if (sid === sessionId) {
        this.sessions.delete(targetId)
        return
      }
    }
  }

  private async attach(targetId: string, pageId: number): Promise<SessionId> {
    await this.ensureConnected()
    const cached = this.sessions.get(targetId)
    if (cached) return cached

    const { sessionId } = await this.cdp.Target.attachToTarget({
      targetId,
      flatten: true,
    })
    const session = this.cdp.session(sessionId)
    await Promise.all([
      session.Page.enable(),
      session.DOM.enable(),
      session.Runtime.enable(),
      session.Accessibility.enable(),
    ])
    this.sessions.set(targetId, sessionId)
    await this.hooks.onSessionAttached?.(session, pageId, sessionId)
    return sessionId
  }

  private async ensureConnected(): Promise<boolean> {
    if (!this.cdp.isConnected()) {
      await this.waitForConnection()
    }

    const epoch = this.cdp.connectionEpoch()
    if (epoch !== this.connectionEpoch) {
      this.sessions.clear()
      this.hiddenWindowId = undefined
      this.connectionEpoch = epoch
      return true
    }
    return false
  }

  private async waitForConnection(): Promise<void> {
    const deadline = Date.now() + 5000
    while (!this.cdp.isConnected() && Date.now() < deadline) {
      await delay(50)
    }
    if (!this.cdp.isConnected()) throw new Error('CDP not connected')
  }

  private async ensurePageIdForTarget(targetId: string): Promise<number> {
    const existing = this.findByTarget(targetId)
    if (existing) return existing.pageId

    await this.list()
    const found = this.findByTarget(targetId)
    if (found) return found.pageId

    throw new Error(`Could not resolve pageId for target ${targetId}`)
  }

  private findByTarget(targetId: string): PageInfo | undefined {
    for (const info of this.pages.values()) {
      if (info.targetId === targetId) return info
    }
    return undefined
  }

  private findByTab(tabId: number): PageInfo | undefined {
    for (const info of this.pages.values()) {
      if (info.tabId === tabId) return info
    }
    return undefined
  }

  private requireInfo(pageId: number): PageInfo {
    const info = this.pages.get(pageId)
    if (!info) {
      throw new Error(`Unknown page ${pageId}. List pages to see what is open.`)
    }
    return info
  }

  private updateFromTab(pageId: number, tab: TabInfo): PageInfo {
    const info = this.requireInfo(pageId)
    const updated: PageInfo = {
      ...info,
      ...tab,
      windowId: tab.windowId ?? info.windowId,
    }
    this.pages.set(pageId, updated)
    return updated
  }
}
