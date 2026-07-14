import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { CdpBackend } from './backends/types'
import type { PageInfo } from './core/pages'
import { BrowserSession } from './core/session'
import { logger } from './logger'

export type { PageInfo } from './core/pages'

/** Server facade over BrowserSession for callers that are not MCP tools. */
export class Browser {
  private core: BrowserSession

  constructor(cdp: CdpBackend) {
    this.core = new BrowserSession(cdp)
  }

  isCdpConnected(): boolean {
    return this.core.isConnected()
  }

  /** Browser-core session shared by MCP and the in-process agent. */
  get session(): BrowserSession {
    return this.core
  }

  /** Resolves a window's active page to the CDP session used by screencast. */
  async getActivePageForWindow(windowId: number): Promise<{
    targetId: string
    session: ProtocolApi
    url: string
  }> {
    return this.core.pages.getActiveSessionForWindow(windowId)
  }

  /** Resolves a BrowserOS page id to the CDP session used by screencast. */
  async getPageSession(pageId: number): Promise<{
    targetId: string
    session: ProtocolApi
    url: string
  }> {
    return this.core.pages.getSession(pageId)
  }

  async listPages(): Promise<PageInfo[]> {
    return this.core.pages.list()
  }

  async newPage(
    url: string,
    opts?: { hidden?: boolean; background?: boolean; windowId?: number },
  ): Promise<number> {
    if (opts?.hidden) return this.core.pages.newPage(url, opts)
    const windowId = await this.resolveVisibleWindowId(opts?.windowId)
    return this.core.pages.newPage(url, {
      background: opts?.background,
      windowId,
    })
  }

  async closePage(page: number): Promise<void> {
    await this.core.pages.close(page)
  }

  async resolveTabIds(tabIds: number[]): Promise<Map<number, number>> {
    return this.core.pages.resolveTabIds(tabIds)
  }

  private async resolveVisibleWindowId(
    requestedWindowId?: number,
  ): Promise<number | undefined> {
    if (requestedWindowId !== undefined) return requestedWindowId

    const windows = await this.core.windows.list()
    const visibleWindow =
      windows.find((window) => window.isVisible && window.isActive) ??
      windows.find((window) => window.isVisible)
    if (visibleWindow) return visibleWindow.windowId

    logger.warn('No visible browser window found; creating one for new page')
    return (await this.core.windows.create({ hidden: false })).windowId
  }
}
