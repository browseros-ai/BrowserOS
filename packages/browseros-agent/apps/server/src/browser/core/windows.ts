import type { WindowInfo } from '@browseros/cdp-protocol/domains/browser'
import type { CdpConnection } from './connection'

export type { WindowInfo }

export interface SetWindowVisibilityResult {
  window: WindowInfo
  replaced: boolean
  previousWindowId: number
  newWindowId: number
}

const delay = (ms: number): Promise<void> =>
  new Promise((resolve) => setTimeout(resolve, ms))

/** Wraps BrowserOS window CDP commands for browser-core callers and tools. */
export class WindowManager {
  constructor(private readonly cdp: CdpConnection) {}

  async list(): Promise<WindowInfo[]> {
    await this.ensureConnected()
    const result = await this.cdp.Browser.getWindows()
    return result.windows as WindowInfo[]
  }

  async create(opts?: { hidden?: boolean }): Promise<WindowInfo> {
    await this.ensureConnected()
    const result = await this.cdp.Browser.createWindow({
      hidden: opts?.hidden ?? false,
    })
    return result.window as WindowInfo
  }

  async close(windowId: number): Promise<void> {
    await this.ensureConnected()
    await this.cdp.Browser.closeWindow({ windowId })
  }

  async activate(windowId: number): Promise<void> {
    await this.ensureConnected()
    await this.cdp.Browser.activateWindow({ windowId })
  }

  /** Changes visibility and returns the replacement window ID when BrowserOS swaps windows. */
  async setVisibility(
    windowId: number,
    opts: { visible: boolean; activate?: boolean },
  ): Promise<SetWindowVisibilityResult> {
    await this.ensureConnected()
    const result = await this.cdp.Browser.setWindowVisibility({
      windowId,
      visible: opts.visible,
      ...(opts.activate !== undefined && { activate: opts.activate }),
    })
    const window = result.window as WindowInfo
    return {
      window,
      replaced: result.replaced,
      previousWindowId: result.previousWindowId,
      newWindowId: window.windowId,
    }
  }

  private async ensureConnected(): Promise<void> {
    if (this.cdp.isConnected()) return

    const deadline = Date.now() + 5000
    while (!this.cdp.isConnected() && Date.now() < deadline) {
      await delay(50)
    }
    if (!this.cdp.isConnected()) throw new Error('CDP not connected')
  }
}
