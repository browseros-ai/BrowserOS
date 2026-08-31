import {
  type ConversationPanel,
  type ConversationPanelSnapshot,
  ConversationPanelSnapshotSchema,
} from '@browseros/shared/schemas/conversation-panels'
import { EventSourceParserStream } from 'eventsource-parser/stream'
import type { GlowMessage } from '@/entrypoints/glow.content/GlowMessage'
import type { ConversationPanelViews } from './conversationPanelStorage'

export interface ConversationPanelBrokerDeps {
  resolveServerUrl(): Promise<string>
  fetch(input: string, init: RequestInit): Promise<Response>
  getTab(tabId: number): Promise<{ id?: number; windowId: number }>
  openPanel(target: { tabId: number; windowId: number }): Promise<void>
  readViews(): Promise<ConversationPanelViews>
  writeViews(views: ConversationPanelViews): Promise<void>
  sendGlow(tabId: number, message: GlowMessage): Promise<void> | void
  hasShownConfetti(): Promise<boolean>
  markConfettiShown(): Promise<void>
  wait?(milliseconds: number, signal: AbortSignal): Promise<void>
}

/**
 * Bridges server conversation state into browser UI routing. It is the only
 * extension component that opens panels or mutates tab-to-conversation state;
 * React panels merely render the mapping it publishes.
 */
export class ConversationPanelBroker {
  private views: ConversationPanelViews = {}
  private loadPromise: Promise<void> | undefined
  private loopPromise: Promise<void> | undefined
  private connectAbort: AbortController | undefined
  private stopped = true
  private hasServerSnapshot = false

  constructor(private readonly deps: ConversationPanelBrokerDeps) {}

  start(): Promise<void> {
    if (this.loopPromise) return this.loopPromise
    this.stopped = false
    this.loopPromise = this.runReconnectLoop().finally(() => {
      this.loopPromise = undefined
    })
    return this.loopPromise
  }

  stop(): void {
    this.stopped = true
    this.connectAbort?.abort()
  }

  /** Reconciles one authoritative server snapshot in stream order. */
  async accept(snapshot: ConversationPanelSnapshot): Promise<void> {
    await this.ensureLoaded()

    const previous = this.views
    const firstServerSnapshot = !this.hasServerSnapshot
    const next = Object.fromEntries(
      snapshot.tabs.map((tab) => [String(tab.tabId), tab]),
    )
    const stopped = Object.values(previous).filter((panel) => {
      if (panel.status !== 'running') return false
      const replacement = next[String(panel.tabId)]
      return !sameRunningPanel(panel, replacement)
    })
    const started = snapshot.tabs.filter((panel) => {
      if (panel.status !== 'running') return false
      return (
        firstServerSnapshot ||
        !sameRunningPanel(previous[String(panel.tabId)], panel)
      )
    })

    // Storage is a browser-process cache, never a second source of truth. Write
    // the complete projection before asynchronous Chrome side effects run.
    this.views = next
    this.hasServerSnapshot = true
    await this.persistViews()
    await this.deactivate(stopped, next)
    for (const panel of started) await this.activate(panel)
  }

  private async deactivate(
    stopped: ConversationPanel[],
    next: ConversationPanelViews,
  ): Promise<void> {
    const completed = stopped.filter((panel) => {
      const replacement = next[String(panel.tabId)]
      return (
        replacement?.conversationId === panel.conversationId &&
        replacement.runId === panel.runId &&
        replacement.status === 'completed'
      )
    })
    const showCompletion =
      completed.length > 0 && !(await this.deps.hasShownConfetti())
    const confettiTabId = completed[0]?.tabId

    for (const panel of stopped) {
      await this.deps.sendGlow(panel.tabId, {
        conversationId: panel.conversationId,
        isActive: false,
        ...(showCompletion && {
          showConfetti: panel.tabId === confettiTabId,
        }),
      })
    }
    if (showCompletion) await this.deps.markConfettiShown()
  }

  private async runReconnectLoop(): Promise<void> {
    let retryDelayMs = 250
    while (!this.stopped) {
      this.connectAbort = new AbortController()
      try {
        await this.consumeSnapshots(this.connectAbort.signal)
        retryDelayMs = 250
      } catch {
        if (this.stopped || this.connectAbort.signal.aborted) break
      }
      if (this.stopped) break
      await (this.deps.wait ?? waitFor)(
        retryDelayMs,
        this.connectAbort.signal,
      ).catch(() => undefined)
      retryDelayMs = Math.min(retryDelayMs * 2, 5_000)
    }
  }

  private async consumeSnapshots(signal: AbortSignal): Promise<void> {
    const serverUrl = await this.deps.resolveServerUrl()
    const response = await this.deps.fetch(`${serverUrl}/chat/panels`, {
      headers: { Accept: 'text/event-stream' },
      signal,
    })
    if (!response.ok || !response.body) {
      throw new Error(`Conversation panels unavailable (${response.status})`)
    }

    // The parser buffers arbitrary fetch chunk boundaries; each parsed event is
    // awaited so storage and panel effects preserve server ordering.
    const events = response.body
      .pipeThrough(new TextDecoderStream())
      .pipeThrough(new EventSourceParserStream())
    for await (const message of events) {
      let decoded: unknown
      try {
        decoded = JSON.parse(message.data)
      } catch {
        continue
      }
      const snapshot = ConversationPanelSnapshotSchema.safeParse(decoded)
      if (snapshot.success) await this.accept(snapshot.data)
    }
  }

  private async activate(tab: ConversationPanel): Promise<void> {
    try {
      const browserTab = await this.deps.getTab(tab.tabId)
      await this.deps.openPanel({
        tabId: tab.tabId,
        windowId: browserTab.windowId,
      })
    } catch {
      // The server may report an effect immediately before a tab closes. The
      // retained mapping is harmless and can still be replaced by a later run.
      return
    }
    await this.deps.sendGlow(tab.tabId, {
      conversationId: tab.conversationId,
      isActive: true,
    })
  }

  private async ensureLoaded(): Promise<void> {
    this.loadPromise ??= this.deps.readViews().then((views) => {
      this.views = views
    })
    await this.loadPromise
  }

  private async persistViews(): Promise<void> {
    await this.deps.writeViews({ ...this.views })
  }
}

function sameRunningPanel(
  left: ConversationPanel | undefined,
  right: ConversationPanel | undefined,
): boolean {
  return (
    left?.status === 'running' &&
    right?.status === 'running' &&
    left.conversationId === right.conversationId &&
    left.runId === right.runId
  )
}

function waitFor(milliseconds: number, signal: AbortSignal): Promise<void> {
  return new Promise((resolve, reject) => {
    if (signal.aborted) {
      reject(signal.reason)
      return
    }
    const timeout = setTimeout(resolve, milliseconds)
    signal.addEventListener(
      'abort',
      () => {
        clearTimeout(timeout)
        reject(signal.reason)
      },
      { once: true },
    )
  })
}
