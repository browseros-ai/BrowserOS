import type { ProtocolApi } from '@browseros/cdp-protocol/protocol-api'
import type { CdpConnection } from './connection'
import { Input } from './input/input'
import { Navigation } from './navigation'
import { FrameRegistry } from './observer/frames'
import { Observer } from './observer/observer'
import { PageManager, type PageManagerHooks } from './pages'
import {
  captureScreenshotWithAnnotations,
  type ScreenshotCaptureOptions,
  type ScreenshotCaptureResult,
} from './screenshot'
import { WindowManager } from './windows'

export interface BrowserSessionHooks extends PageManagerHooks {}

/** Coordinates page registry, observation, input, navigation, and raw CDP access. */
export class BrowserSession {
  readonly pages: PageManager
  readonly windows: WindowManager
  private readonly frames: FrameRegistry
  private readonly observers = new Map<number, Observer>()

  constructor(
    private readonly connection: CdpConnection,
    hooks: BrowserSessionHooks = {},
  ) {
    this.frames = new FrameRegistry(connection)
    this.windows = new WindowManager(connection)
    this.pages = new PageManager(connection, {
      ...hooks,
      onSessionAttached: async (session, pageId, sessionId) => {
        await this.frames.registerPage(session, pageId, sessionId)
        await hooks.onSessionAttached?.(session, pageId, sessionId)
      },
    })
    this.connection.Target.on('detachedFromTarget', (params) => {
      if (params.sessionId) this.pages.detachSession(params.sessionId)
    })
  }

  /** Per-page observation (snapshot + diff), created lazily and cached. */
  observe(pageId: number): Observer {
    let observer = this.observers.get(pageId)
    if (!observer) {
      observer = new Observer(this.pages, this.frames, pageId)
      this.observers.set(pageId, observer)
    }
    return observer
  }

  /** The action layer (click/fill/type/...) for a page, sharing its observation refs. */
  input(pageId: number): Input {
    return new Input(this.observe(pageId), this.pages, pageId)
  }

  /** Navigation (url/back/forward/reload) for a page. */
  nav(pageId: number): Navigation {
    return new Navigation(this.pages, pageId)
  }

  /** Captures a page screenshot, optionally overlaying current snapshot refs. */
  async screenshot(
    pageId: number,
    options: ScreenshotCaptureOptions = {},
  ): Promise<ScreenshotCaptureResult> {
    const { session } = await this.pages.getSession(pageId)
    return captureScreenshotWithAnnotations({
      pageSession: session,
      observer: this.observe(pageId),
      options,
    })
  }

  /** Captures only while `pageId` still resolves to the expected CDP target. */
  async screenshotForTarget(
    pageId: number,
    targetId: string,
    options: ScreenshotCaptureOptions = {},
  ): Promise<ScreenshotCaptureResult | null> {
    const selected = this.pages.getInfo(pageId)
    if (!selected || selected.targetId !== targetId) return null
    let resolved: Awaited<ReturnType<PageManager['getSession']>>
    try {
      resolved = await this.pages.getSession(pageId)
    } catch (error) {
      const current = this.pages.getInfo(pageId)
      if (!current || current.targetId !== targetId) return null
      throw error
    }
    const { session, targetId: currentTargetId } = resolved
    if (currentTargetId !== targetId) return null
    const capture = await captureScreenshotWithAnnotations({
      pageSession: session,
      observer: this.observe(pageId),
      options,
    })
    const current = await this.pages.refresh(pageId)
    return current?.targetId === targetId ? capture : null
  }

  /** Raw CDP escape hatch for `run` code, e.g. cdp("Page.navigate", { url }). */
  async cdp(
    method: string,
    params?: Record<string, unknown>,
    sessionId?: string,
  ): Promise<unknown> {
    return this.connection.rawSend(method, params ?? {}, sessionId)
  }

  /** Raw CDP escape hatch that sends already-validated JSON params verbatim. */
  async cdpJson(
    method: string,
    paramsJson: string,
    sessionId?: string,
  ): Promise<unknown> {
    return this.connection.rawSendJson(method, paramsJson, sessionId)
  }

  /** Page-scoped raw CDP for CLI/run callers that start from a BrowserOS page id. */
  async cdpJsonForPage(
    pageId: number,
    method: string,
    paramsJson: string,
  ): Promise<unknown> {
    const { sessionId } = await this.pages.getSession(pageId)
    return this.connection.rawSendJson(method, paramsJson, sessionId)
  }

  isConnected(): boolean {
    return this.connection.isConnected()
  }

  /** Exposes the typed root CDP domains for server-side lifecycle integrations. */
  get protocol(): ProtocolApi {
    return this.connection
  }

  /** Identifies reconnects so lifecycle consumers can rebuild cached browser state. */
  connectionEpoch(): number {
    return this.connection.connectionEpoch()
  }

  async dispose(): Promise<void> {}
}
