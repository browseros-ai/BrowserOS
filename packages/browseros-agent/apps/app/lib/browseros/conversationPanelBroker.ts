import {
  type ConversationPresenceEvent,
  ConversationPresenceEventSchema,
  type ConversationPresenceRun,
  type TabConversationPresence,
} from '@browseros/shared/schemas/conversation-presence'
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
 * Bridges server conversation presence into browser UI routing. It is the only
 * extension component that opens panels or mutates tab-to-conversation state;
 * React panels merely render the mapping it publishes.
 */
export class ConversationPanelBroker {
  private views: ConversationPanelViews = {}
  private loadPromise: Promise<void> | undefined
  private loopPromise: Promise<void> | undefined
  private connectAbort: AbortController | undefined
  private stopped = true

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

  /** Applies one validated server event in stream order. */
  async accept(event: ConversationPresenceEvent): Promise<void> {
    await this.ensureLoaded()

    switch (event.type) {
      case 'snapshot': {
        const nextViews = Object.fromEntries(
          event.tabs.map((tab) => [String(tab.tabId), tab]),
        )
        const staleRunning = Object.values(this.views).filter((tab) => {
          const next = nextViews[String(tab.tabId)]
          return (
            tab.status === 'running' &&
            (!next ||
              next.conversationId !== tab.conversationId ||
              next.runId !== tab.runId)
          )
        })
        this.views = nextViews
        await this.persistViews()
        for (const tab of staleRunning) {
          await this.deps.sendGlow(tab.tabId, {
            conversationId: tab.conversationId,
            isActive: false,
          })
        }
        for (const tab of event.tabs) {
          if (tab.status === 'running') await this.activate(tab)
        }
        return
      }
      case 'run-started': {
        const tabs = event.run.tabIds.map((tabId) =>
          tabViewFromRun(tabId, event.run),
        )
        for (const tab of tabs) this.views[String(tab.tabId)] = tab
        await this.persistViews()
        for (const tab of tabs) await this.activate(tab)
        return
      }
      case 'tab-touched': {
        this.views[String(event.tab.tabId)] = event.tab
        await this.persistViews()
        await this.activate(event.tab)
        return
      }
      case 'run-finished': {
        const currentTabs = event.run.tabIds
          .map((tabId) => this.views[String(tabId)])
          .filter(
            (tab): tab is TabConversationPresence =>
              tab?.conversationId === event.run.conversationId &&
              tab.runId === event.run.runId,
          )
        for (const tab of currentTabs) {
          this.views[String(tab.tabId)] = {
            ...tab,
            status: event.run.status,
            updatedAt: event.run.updatedAt,
          }
        }
        await this.persistViews()
        await this.deactivate(event.run, currentTabs)
        return
      }
      case 'conversation-forgotten': {
        for (const tabId of event.tabIds) {
          const current = this.views[String(tabId)]
          if (current?.conversationId === event.conversationId) {
            delete this.views[String(tabId)]
          }
        }
        await this.persistViews()
      }
    }
  }

  private async runReconnectLoop(): Promise<void> {
    let retryDelayMs = 250
    while (!this.stopped) {
      this.connectAbort = new AbortController()
      try {
        await this.consumePresence(this.connectAbort.signal)
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

  private async consumePresence(signal: AbortSignal): Promise<void> {
    const serverUrl = await this.deps.resolveServerUrl()
    const response = await this.deps.fetch(`${serverUrl}/chat/presence`, {
      headers: { Accept: 'text/event-stream' },
      signal,
    })
    if (!response.ok || !response.body) {
      throw new Error(`Conversation presence unavailable (${response.status})`)
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
      const event = ConversationPresenceEventSchema.safeParse(decoded)
      if (event.success) await this.accept(event.data)
    }
  }

  private async activate(tab: TabConversationPresence): Promise<void> {
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

  private async deactivate(
    run: ConversationPresenceRun,
    tabs: TabConversationPresence[],
  ): Promise<void> {
    const showCompletion =
      run.status === 'completed' &&
      tabs.length > 0 &&
      !(await this.deps.hasShownConfetti())
    for (const [index, tab] of tabs.entries()) {
      await this.deps.sendGlow(tab.tabId, {
        conversationId: run.conversationId,
        isActive: false,
        showConfetti: showCompletion && index === 0,
      })
    }
    if (showCompletion) await this.deps.markConfettiShown()
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

function tabViewFromRun(
  tabId: number,
  run: ConversationPresenceRun,
): TabConversationPresence {
  return {
    tabId,
    conversationId: run.conversationId,
    runId: run.runId,
    status: run.status,
    updatedAt: run.updatedAt,
  }
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
