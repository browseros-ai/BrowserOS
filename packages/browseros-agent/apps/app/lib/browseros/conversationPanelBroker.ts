import {
  type ConversationPanelAssignment,
  type ConversationPanelAssignments,
  ConversationPanelAssignmentsSchema,
} from '@browseros/shared/schemas/conversation-panels'
import { EventSourceParserStream } from 'eventsource-parser/stream'
import type { GlowMessage } from '@/entrypoints/glow.content/GlowMessage'
import type { ConversationPanelViews } from './conversationPanelStorage'

export interface ConversationPanelBrokerErrorContext {
  phase: 'stream' | 'open-panel' | 'completion-effect'
  tabId?: number
}

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
  reportError?(
    error: unknown,
    context: ConversationPanelBrokerErrorContext,
  ): void
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
  // Panel opening is safe to reassert, but glow activation restarts animation.
  // Track that non-idempotent effect separately from authoritative assignments.
  private readonly activatedRunByTab = new Map<number, string>()
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

  /** Makes local storage and browser effects match one complete assignment set. */
  async reconcile(assignments: ConversationPanelAssignments): Promise<void> {
    await this.ensureLoaded()

    const previous = this.views
    const next = Object.fromEntries(
      assignments.assignments.map((assignment) => [
        String(assignment.tabId),
        assignment,
      ]),
    )
    const stopped = Object.values(previous).filter((assignment) => {
      if (assignment.status !== 'running') return false
      const replacement = next[String(assignment.tabId)]
      return !sameRunningAssignment(assignment, replacement)
    })
    const running = assignments.assignments.filter(
      (assignment) => assignment.status === 'running',
    )

    // Storage is the handoff to independently mounted React panels. Commit it
    // before opening anything, but do not advance memory if the write fails:
    // the next heartbeat must still see the old state and retry the full handoff.
    await this.deps.writeViews(next)
    this.views = next

    for (const assignment of stopped) {
      this.activatedRunByTab.delete(assignment.tabId)
    }
    await this.deactivate(stopped, next)
    // `open: true` is idempotent. Reassert every running assignment so an SSE
    // reconnect or heartbeat repairs a transient Chrome-side failure.
    for (const assignment of running) await this.activate(assignment)
  }

  private async deactivate(
    stopped: ConversationPanelAssignment[],
    next: ConversationPanelViews,
  ): Promise<void> {
    const completed = stopped.filter((assignment) => {
      const replacement = next[String(assignment.tabId)]
      return (
        replacement?.conversationId === assignment.conversationId &&
        replacement.runId === assignment.runId &&
        replacement.status === 'completed'
      )
    })
    let showCompletion = false
    if (completed.length > 0) {
      try {
        showCompletion = !(await this.deps.hasShownConfetti())
      } catch (error) {
        this.reportError(error, { phase: 'completion-effect' })
      }
    }
    const confettiTabId = completed[0]?.tabId

    for (const assignment of stopped) {
      try {
        await this.deps.sendGlow(assignment.tabId, {
          conversationId: assignment.conversationId,
          isActive: false,
          ...(showCompletion && {
            showConfetti: assignment.tabId === confettiTabId,
          }),
        })
      } catch (error) {
        this.reportError(error, {
          phase: 'completion-effect',
          tabId: assignment.tabId,
        })
      }
    }
    if (showCompletion) {
      try {
        await this.deps.markConfettiShown()
      } catch (error) {
        this.reportError(error, { phase: 'completion-effect' })
      }
    }
  }

  private async runReconnectLoop(): Promise<void> {
    let retryDelayMs = 250
    while (!this.stopped) {
      this.connectAbort = new AbortController()
      try {
        await this.consumeAssignments(this.connectAbort.signal)
        retryDelayMs = 250
      } catch (error) {
        if (this.stopped || this.connectAbort.signal.aborted) break
        this.reportError(error, { phase: 'stream' })
      }
      if (this.stopped) break
      await (this.deps.wait ?? waitFor)(
        retryDelayMs,
        this.connectAbort.signal,
      ).catch(() => undefined)
      retryDelayMs = Math.min(retryDelayMs * 2, 5_000)
    }
  }

  private async consumeAssignments(signal: AbortSignal): Promise<void> {
    const serverUrl = await this.deps.resolveServerUrl()
    const response = await this.deps.fetch(`${serverUrl}/chat/panels`, {
      headers: { Accept: 'text/event-stream' },
      signal,
    })
    if (!response.ok || !response.body) {
      throw new Error(`Conversation panels unavailable (${response.status})`)
    }

    // The parser buffers arbitrary fetch chunk boundaries. Each full assignment
    // set is reconciled in stream order, including periodic healing heartbeats.
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
      const assignments = ConversationPanelAssignmentsSchema.safeParse(decoded)
      if (assignments.success) await this.reconcile(assignments.data)
    }
  }

  private async activate(
    assignment: ConversationPanelAssignment,
  ): Promise<void> {
    try {
      const browserTab = await this.deps.getTab(assignment.tabId)
      await this.deps.openPanel({
        tabId: assignment.tabId,
        windowId: browserTab.windowId,
      })
    } catch (error) {
      // The server may report an effect immediately before a tab closes. The
      // retained mapping is harmless; a heartbeat retries other transient errors.
      this.reportError(error, {
        phase: 'open-panel',
        tabId: assignment.tabId,
      })
      return
    }

    if (this.activatedRunByTab.get(assignment.tabId) === assignment.runId) {
      return
    }
    await this.deps.sendGlow(assignment.tabId, {
      conversationId: assignment.conversationId,
      isActive: true,
    })
    this.activatedRunByTab.set(assignment.tabId, assignment.runId)
  }

  private async ensureLoaded(): Promise<void> {
    if (!this.loadPromise) {
      const loading = this.deps.readViews().then((views) => {
        // Session storage carries the last assignment set across MV3 worker
        // suspension, allowing reconciliation to stop glows from a finished run.
        this.views = views
      })
      this.loadPromise = loading
      try {
        await loading
      } catch (error) {
        // Storage can be briefly unavailable during extension startup. A failed
        // read must not poison every later SSE reconnect with the same promise.
        if (this.loadPromise === loading) this.loadPromise = undefined
        throw error
      }
      return
    }
    await this.loadPromise
  }

  private reportError(
    error: unknown,
    context: ConversationPanelBrokerErrorContext,
  ): void {
    try {
      this.deps.reportError?.(error, context)
    } catch {
      // Diagnostics must never become another failure in the reconciliation path.
    }
  }
}

function sameRunningAssignment(
  left: ConversationPanelAssignment | undefined,
  right: ConversationPanelAssignment | undefined,
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
