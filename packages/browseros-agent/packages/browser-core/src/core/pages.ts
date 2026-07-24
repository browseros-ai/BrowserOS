import type { TabInfo as ProtocolTabInfo } from '@browseros/cdp-protocol/domains/browser'
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
  windowId?: number
  index?: number
  groupId?: string
}

export interface PageSession {
  targetId: string
  sessionId: string
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

  constructor(
    private readonly cdp: CdpConnection,
    private readonly hooks: PageManagerHooks = {},
  ) {
    this.connectionEpoch = cdp.connectionEpoch()
  }

  /** Reconcile the registry with the browser's live tabs (upsert + drop vanished). */
  async list(): Promise<PageInfo[]> {
    await this.ensureConnected()
    const result = await this.cdp.Browser.getTabs()
    const tabs = (result.tabs as ProtocolTabInfo[]).filter(
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
        this.pages.set(
          existing.pageId,
          pageInfoFromProtocol(existing.pageId, tab, {
            windowId: existing.windowId,
          }),
        )
      } else {
        const pageId = this.nextPageId++
        this.pages.set(pageId, pageInfoFromProtocol(pageId, tab))
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
      sessionId,
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
    const tab = result.tab as ProtocolTabInfo
    return this.findByTarget(tab.targetId) ?? null
  }

  async getActiveSessionForWindow(windowId: number): Promise<PageSession> {
    await this.ensureConnected()
    const result = await this.cdp.Browser.getActiveTab({ windowId })
    const tab = result.tab as ProtocolTabInfo | undefined
    if (!tab) throw new Error(`No active tab in window ${windowId}`)

    const pageId = await this.ensurePageIdForTarget(tab.targetId)
    const sessionId = await this.attach(tab.targetId, pageId)
    return {
      targetId: tab.targetId,
      sessionId,
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
    const observed = { ...info }

    try {
      const result = await this.cdp.Browser.getTabInfo({
        tabId: observed.tabId,
      })
      const tab = result.tab as ProtocolTabInfo
      const updated = pageInfoFromProtocol(pageId, tab, {
        windowId: observed.windowId,
      })
      const current = this.pages.get(pageId)
      if (!current) return undefined
      // Browser reconciliation can rebind a stable page ID while CDP is in
      // flight. A stale response must not overwrite the newer incarnation.
      if (!samePageInfo(current, observed)) return current
      if (updated.targetId !== observed.targetId) {
        this.sessions.delete(observed.targetId)
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
      windowId?: number
      tabGroupId?: string
    },
  ): Promise<number> {
    await this.ensureConnected()
    const created = await this.cdp.Browser.createTab({
      url,
      ...(opts?.background !== undefined && { background: opts.background }),
      ...(opts?.windowId !== undefined && { windowId: opts.windowId }),
    })
    const tabId = (created.tab as ProtocolTabInfo).tabId

    let tab: ProtocolTabInfo | undefined
    for (let attempt = 0; attempt < 30; attempt++) {
      try {
        tab = (await this.cdp.Browser.getTabInfo({ tabId }))
          .tab as ProtocolTabInfo
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
        tab = (await this.cdp.Browser.getTabInfo({ tabId }))
          .tab as ProtocolTabInfo
      } catch (error) {
        logger.warn('Failed to add new page to default tab group', {
          tabGroupId: opts.tabGroupId,
          tabId,
          error: error instanceof Error ? error.message : String(error),
        })
      }
    }

    const pageId = this.nextPageId++
    this.pages.set(pageId, pageInfoFromProtocol(pageId, tab, { url }))
    return pageId
  }

  async close(pageId: number): Promise<void> {
    const info = this.pages.get(pageId)
    if (!info) throw new Error(`Unknown page ${pageId}.`)
    await this.cdp.Browser.closeTab({ tabId: info.tabId })
    this.pages.delete(pageId)
    this.sessions.delete(info.targetId)
    this.hooks.onPageDetached?.(pageId)
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
    return this.updateFromTab(pageId, result.tab as ProtocolTabInfo)
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

  private updateFromTab(pageId: number, tab: ProtocolTabInfo): PageInfo {
    const info = this.requireInfo(pageId)
    const updated = pageInfoFromProtocol(pageId, tab, {
      windowId: info.windowId,
    })
    this.pages.set(pageId, updated)
    return updated
  }
}

function samePageInfo(left: PageInfo, right: PageInfo): boolean {
  return (
    left.pageId === right.pageId &&
    left.targetId === right.targetId &&
    left.tabId === right.tabId &&
    left.url === right.url &&
    left.title === right.title &&
    left.isActive === right.isActive &&
    left.isLoading === right.isLoading &&
    left.loadProgress === right.loadProgress &&
    left.isPinned === right.isPinned &&
    left.windowId === right.windowId &&
    left.index === right.index &&
    left.groupId === right.groupId
  )
}

/**
 * Projects the generated wire tab into BrowserOS-owned page state so
 * compatibility-only protocol fields cannot escape through structured results.
 */
function pageInfoFromProtocol(
  pageId: number,
  tab: ProtocolTabInfo,
  fallback: { windowId?: number; url?: string } = {},
): PageInfo {
  const windowId = tab.windowId ?? fallback.windowId
  return {
    pageId,
    targetId: tab.targetId,
    tabId: tab.tabId,
    url: tab.url || fallback.url || '',
    title: tab.title,
    isActive: tab.isActive,
    isLoading: tab.isLoading,
    loadProgress: tab.loadProgress,
    isPinned: tab.isPinned,
    ...(windowId !== undefined && { windowId }),
    ...(tab.index !== undefined && { index: tab.index }),
    ...(tab.groupId !== undefined && { groupId: tab.groupId }),
  }
}
