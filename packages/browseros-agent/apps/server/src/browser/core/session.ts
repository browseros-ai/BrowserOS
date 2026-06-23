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

  /** Raw CDP escape hatch for `run` code, e.g. cdp("Page.navigate", { url }). */
  async cdp(
    method: string,
    params?: Record<string, unknown>,
    sessionId?: string,
  ): Promise<unknown> {
    const api = sessionId ? this.connection.session(sessionId) : this.connection
    const [domain, command] = method.split('.')
    const target = (
      api as unknown as Record<
        string,
        Record<string, (p?: unknown) => Promise<unknown>>
      >
    )[domain]
    if (!target?.[command]) {
      throw new Error(`Unknown CDP method "${method}"`)
    }
    return target[command](params ?? {})
  }

  isConnected(): boolean {
    return this.connection.isConnected()
  }

  async dispose(): Promise<void> {}
}
