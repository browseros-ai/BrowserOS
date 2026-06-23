import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import { logger } from '../lib/logger'
import type { CdpBackend } from './backends/types'
import type { PageInfo } from './core/pages'
import { BrowserSession } from './core/session'

export type { PageInfo } from './core/pages'

/** Server/eval facade over BrowserSession for callers that are not MCP tools. */
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

  private async resolveSession(page: number): Promise<ProtocolApi> {
    return (await this.core.pages.getSession(page)).session
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

  /** Captures a page screenshot and reports DPR for direct eval capture. */
  async screenshot(
    page: number,
    opts: { format: string; quality?: number; fullPage: boolean },
  ): Promise<{ data: string; mimeType: string; devicePixelRatio: number }> {
    const session = await this.resolveSession(page)

    const params: Record<string, unknown> = {
      format: opts.format,
      captureBeyondViewport: opts.fullPage,
    }
    if (opts.quality !== undefined) params.quality = opts.quality

    const [screenshotResult, dprResult] = await Promise.allSettled([
      session.Page.captureScreenshot(
        params as Parameters<ProtocolApi['Page']['captureScreenshot']>[0],
      ),
      session.Runtime.evaluate({
        expression: 'window.devicePixelRatio',
        returnByValue: true,
      }),
    ])

    if (screenshotResult.status === 'rejected') throw screenshotResult.reason

    const result = screenshotResult.value
    const devicePixelRatio =
      dprResult.status === 'fulfilled' &&
      typeof dprResult.value.result?.value === 'number'
        ? dprResult.value.result.value
        : 1

    return {
      data: result.data,
      mimeType: `image/${opts.format}`,
      devicePixelRatio,
    }
  }

  /** Evaluates page JavaScript for direct eval/captcha detection callers. */
  async evaluate(
    page: number,
    expression: string,
  ): Promise<{
    value?: unknown
    error?: string
    description?: string
  }> {
    const session = await this.resolveSession(page)

    const result = await session.Runtime.evaluate({
      expression,
      returnByValue: true,
      awaitPromise: true,
    })

    if (result.exceptionDetails) {
      return {
        error:
          result.exceptionDetails.exception?.description ??
          result.exceptionDetails.text,
      }
    }

    return {
      value: result.result?.value,
      description: result.result?.description,
    }
  }
}
