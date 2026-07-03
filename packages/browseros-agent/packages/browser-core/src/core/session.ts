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

/**
 * CDP methods the agent must not call via the raw escape hatch.
 * Blocks credential/history exfiltration even if the LLM is prompt-injected.
 * Prefix matches: "Storage." blocks all Storage.* commands, etc.
 */
const CDP_BLOCKED_PREFIXES = [
  'Storage.',
  'DOMStorage.',
  'IndexedDB.',
  'History.',
  'Bookmarks.',
  'Network.getCookies',
  'Network.getAllCookies',
  'Network.clearBrowserCookies',
  'Network.clearBrowserCache',
  'Network.getResponseBody',
  'Network.takeResponseBodyAsStream',
  'Audits.',
  'WebAuthn.',
  'Cast.',
]

function assertCdpMethodAllowed(method: string): void {
  for (const prefix of CDP_BLOCKED_PREFIXES) {
    if (method === prefix || method.startsWith(prefix)) {
      throw new Error(
        `CDP method "${method}" is blocked by the BrowserOS security policy`,
      )
    }
  }
}

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

  /** Raw CDP escape hatch for `run` code, e.g. cdp("Page.navigate", { url }). */
  async cdp(
    method: string,
    params?: Record<string, unknown>,
    sessionId?: string,
  ): Promise<unknown> {
    assertCdpMethodAllowed(method)
    return this.connection.rawSend(method, params ?? {}, sessionId)
  }

  /** Raw CDP escape hatch that sends already-validated JSON params verbatim. */
  async cdpJson(
    method: string,
    paramsJson: string,
    sessionId?: string,
  ): Promise<unknown> {
    assertCdpMethodAllowed(method)
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

  async dispose(): Promise<void> {}
}
