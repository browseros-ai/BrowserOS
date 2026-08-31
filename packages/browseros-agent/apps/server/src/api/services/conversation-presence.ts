/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  ConversationPresenceEvent,
  ConversationPresenceRun,
  ConversationRunStatus,
  TabConversationPresence,
} from '@browseros/shared/schemas/conversation-presence'
import type { BrowserTabTouchedEvent } from './mcp/browser-tool-runtime'

export type {
  ConversationPresenceEvent,
  ConversationPresenceRun,
  TabConversationPresence,
} from '@browseros/shared/schemas/conversation-presence'

export interface ConversationPresenceSnapshot {
  runs: ConversationPresenceRun[]
  tabs: TabConversationPresence[]
}

interface MutablePresenceRun {
  conversationId: string
  runId: string
  status: ConversationRunStatus
  tabIds: Set<number>
  updatedAt: number
}

/**
 * Correlates server-owned runs with Chrome tabs and publishes a resumable view
 * to the extension background. Tab mappings intentionally outlive a run so a
 * panel opened later still knows which conversation belongs to that tab.
 */
export class ConversationPresence {
  private readonly runs = new Map<string, MutablePresenceRun>()
  private readonly pendingRuns = new Map<string, MutablePresenceRun>()
  private readonly tabs = new Map<number, TabConversationPresence>()
  private readonly subscribers = new Set<
    ReadableStreamDefaultController<ConversationPresenceEvent>
  >()

  /** Reserves correlation for early tool effects without publishing UI state. */
  prepareRun(input: {
    conversationId: string
    runId: string
    tabIds: readonly number[]
  }): void {
    this.pendingRuns.set(input.conversationId, this.buildRun(input, Date.now()))
  }

  startRun(input: {
    conversationId: string
    runId: string
    tabIds: readonly number[]
  }): void {
    const now = Date.now()
    const pending = this.pendingRuns.get(input.conversationId)
    const associatedTabIds = new Set(
      pending?.runId === input.runId ? pending.tabIds : input.tabIds,
    )
    for (const tabId of input.tabIds) associatedTabIds.add(tabId)
    // A tab remains a view of its latest conversation across turns. Starting a
    // new turn therefore wakes every panel that still belongs to that
    // conversation, not only the panel that submitted the request. Tabs later
    // claimed by another conversation are deliberately excluded.
    for (const tab of this.tabs.values()) {
      if (tab.conversationId === input.conversationId) {
        associatedTabIds.add(tab.tabId)
      }
    }
    const run: MutablePresenceRun = {
      conversationId: input.conversationId,
      runId: input.runId,
      status: 'running',
      tabIds: associatedTabIds,
      updatedAt: now,
    }
    this.pendingRuns.delete(input.conversationId)
    this.runs.set(input.conversationId, run)
    for (const tabId of run.tabIds) {
      this.tabs.set(tabId, toTabView(tabId, run))
    }
    this.publish({ type: 'run-started', run: toRunView(run) })
  }

  touchTab(event: BrowserTabTouchedEvent): boolean {
    if (!event.runId) return false
    const run = this.runs.get(event.conversationId)
    if (run?.runId === event.runId && run.status === 'running') {
      run.tabIds.add(event.tabId)
      run.updatedAt = Date.now()
      const tab = toTabView(event.tabId, run)
      this.tabs.set(event.tabId, tab)
      this.publish({ type: 'tab-touched', tab })
      return true
    }

    const pending = this.pendingRuns.get(event.conversationId)
    if (pending?.runId !== event.runId) return false
    pending.tabIds.add(event.tabId)
    pending.updatedAt = Date.now()
    return true
  }

  finishRun(
    conversationId: string,
    runId: string,
    status: Exclude<ConversationRunStatus, 'running'>,
  ): boolean {
    const pending = this.pendingRuns.get(conversationId)
    if (pending?.runId === runId) {
      this.pendingRuns.delete(conversationId)
      return true
    }

    const run = this.runs.get(conversationId)
    if (!run || run.runId !== runId || run.status !== 'running') return false

    run.status = status
    run.updatedAt = Date.now()
    for (const tabId of run.tabIds) {
      const latest = this.tabs.get(tabId)
      if (latest?.conversationId === conversationId && latest.runId === runId) {
        this.tabs.set(tabId, toTabView(tabId, run))
      }
    }
    this.publish({ type: 'run-finished', run: toRunView(run) })
    return true
  }

  forgetConversation(conversationId: string): void {
    this.pendingRuns.delete(conversationId)
    this.runs.delete(conversationId)
    const tabIds: number[] = []
    for (const [tabId, tab] of this.tabs) {
      if (tab.conversationId !== conversationId) continue
      this.tabs.delete(tabId)
      tabIds.push(tabId)
    }
    this.publish({
      type: 'conversation-forgotten',
      conversationId,
      tabIds: tabIds.sort((a, b) => a - b),
    })
  }

  snapshot(): ConversationPresenceSnapshot {
    return {
      runs: [...this.runs.values()]
        .map(toRunView)
        .sort((a, b) => a.conversationId.localeCompare(b.conversationId)),
      tabs: [...this.tabs.values()].sort((a, b) => a.tabId - b.tabId),
    }
  }

  /** Snapshot-before-subscribe is atomic because both operations are synchronous. */
  subscribe(): ReadableStream<ConversationPresenceEvent> {
    let subscriber:
      | ReadableStreamDefaultController<ConversationPresenceEvent>
      | undefined
    return new ReadableStream<ConversationPresenceEvent>({
      start: (controller) => {
        subscriber = controller
        controller.enqueue({ type: 'snapshot', ...this.snapshot() })
        this.subscribers.add(controller)
      },
      cancel: () => {
        if (subscriber) this.subscribers.delete(subscriber)
      },
    })
  }

  private publish(event: ConversationPresenceEvent): void {
    for (const subscriber of [...this.subscribers]) {
      try {
        subscriber.enqueue(event)
      } catch {
        this.subscribers.delete(subscriber)
      }
    }
  }

  private buildRun(
    input: {
      conversationId: string
      runId: string
      tabIds: readonly number[]
    },
    updatedAt: number,
  ): MutablePresenceRun {
    const tabIds = new Set(input.tabIds)
    for (const tab of this.tabs.values()) {
      if (tab.conversationId === input.conversationId) tabIds.add(tab.tabId)
    }
    return {
      conversationId: input.conversationId,
      runId: input.runId,
      status: 'running',
      tabIds,
      updatedAt,
    }
  }
}

function toRunView(run: MutablePresenceRun): ConversationPresenceRun {
  return {
    conversationId: run.conversationId,
    runId: run.runId,
    status: run.status,
    tabIds: [...run.tabIds].sort((a, b) => a - b),
    updatedAt: run.updatedAt,
  }
}

function toTabView(
  tabId: number,
  run: MutablePresenceRun,
): TabConversationPresence {
  return {
    tabId,
    conversationId: run.conversationId,
    runId: run.runId,
    status: run.status,
    updatedAt: run.updatedAt,
  }
}
