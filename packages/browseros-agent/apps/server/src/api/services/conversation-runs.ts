/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import type {
  ConversationPanelAssignments,
  ConversationRunStatus,
} from '@browseros/shared/schemas/conversation-panels'
import type { UIMessage, UIMessageChunk } from 'ai'

export type { ConversationRunStatus } from '@browseros/shared/schemas/conversation-panels'

const DEFAULT_PANEL_ASSIGNMENTS_HEARTBEAT_MS = 5_000

export interface ConversationRunSnapshot {
  conversationId: string
  runId: string
  status: ConversationRunStatus
  messages: UIMessage[]
  chunkCount: number
}

export interface ConversationTabGroupPresentation {
  /** Stable title used if this conversation creates browser tabs. */
  title: string
  /** Stable identity input used to choose a Chrome tab-group color. */
  colorKey: string
}

export interface StartConversationRunInput {
  conversationId: string
  messages: UIMessage[]
  /** Chrome tab ids whose panels should display this run immediately. */
  panelTabIds?: readonly number[]
  /** Scheduled work executes normally but must not open or group visible tabs. */
  panelsVisible?: boolean
  tabGroup?: ConversationTabGroupPresentation
  createStream: (
    signal: AbortSignal,
    runId: string,
    updateMessages: (messages: UIMessage[]) => boolean,
  ) => ReadableStream<UIMessageChunk> | Promise<ReadableStream<UIMessageChunk>>
}

export interface StartedConversationRun {
  runId: string
}

/**
 * A request-scoped capability pinned to the run that authorized a tool call.
 * Its mutations become no-ops after that run finishes or is replaced.
 */
export interface ActiveConversationRun {
  readonly conversationId: string
  readonly runId: string
  readonly panelsVisible: boolean
  readonly tabGroup: ConversationTabGroupPresentation | undefined
  /** Aborts browser work when the user stops this exact run. */
  readonly signal: AbortSignal
  associateTabs(tabIds: readonly number[]): boolean
}

export interface ConversationRunActivity {
  beginChatStream(): void
  endChatStream(): void
}

interface ConversationRunRecord {
  conversationId: string
  runId: string
  status: ConversationRunStatus
  messages: UIMessage[]
  chunks: UIMessageChunk[]
  panelsVisible: boolean
  tabGroup?: ConversationTabGroupPresentation
  abortController: AbortController
  reader?: ReadableStreamDefaultReader<UIMessageChunk>
  subscribers: Set<ReadableStreamDefaultController<UIMessageChunk>>
  sawErrorChunk: boolean
  ended: boolean
  deleting: boolean
  activityStarted: boolean
  preparedResolved: boolean
  prepared: Promise<void>
  resolvePrepared: () => void
  finished: Promise<void>
  resolveFinished: () => void
}

interface PanelAssignmentsSubscriber {
  controller: ReadableStreamDefaultController<ConversationPanelAssignments>
  heartbeat?: ReturnType<typeof setInterval>
}

interface ConversationRunsDeps {
  activity?: ConversationRunActivity
  /** Lower values heal clients faster at the cost of more loopback traffic. */
  panelAssignmentsHeartbeatMs?: number
}

export class ConversationRunAlreadyActiveError extends Error {
  constructor() {
    super('A conversation run is already active')
    this.name = 'ConversationRunAlreadyActiveError'
  }
}

export class ConversationRunNotFoundError extends Error {
  constructor() {
    super('Conversation run not found')
    this.name = 'ConversationRunNotFoundError'
  }
}

/**
 * Owns execution, replay, cancellation, and the panel projection for every
 * conversation. Panel entries point directly at run records, so status cannot
 * drift between a separate “presence” service and the stream being rendered.
 */
export class ConversationRuns {
  private readonly runs = new Map<string, ConversationRunRecord>()
  private readonly panelByTab = new Map<number, ConversationRunRecord>()
  private readonly panelSubscribers = new Set<PanelAssignmentsSubscriber>()

  constructor(private readonly deps: ConversationRunsDeps = {}) {}

  async start(
    input: StartConversationRunInput,
  ): Promise<StartedConversationRun> {
    const existing = this.runs.get(input.conversationId)
    if (existing?.status === 'running' || existing?.deleting) {
      throw new ConversationRunAlreadyActiveError()
    }

    let resolveFinished!: () => void
    let resolvePrepared!: () => void
    const prepared = new Promise<void>((resolve) => {
      resolvePrepared = resolve
    })
    const finished = new Promise<void>((resolve) => {
      resolveFinished = resolve
    })
    const record: ConversationRunRecord = {
      conversationId: input.conversationId,
      runId: crypto.randomUUID(),
      status: 'running',
      messages: [...input.messages],
      chunks: [],
      panelsVisible: input.panelsVisible ?? true,
      // Keep the first useful presentation across turns. If no tab was opened
      // until a later turn, its eventual group still describes the conversation
      // rather than only the latest prompt.
      tabGroup: existing?.tabGroup ?? input.tabGroup,
      abortController: new AbortController(),
      subscribers: new Set(),
      sawErrorChunk: false,
      ended: false,
      deleting: false,
      activityStarted: false,
      preparedResolved: false,
      prepared,
      resolvePrepared,
      finished,
      resolveFinished,
    }

    // Install before any provider/MCP preparation awaits. This both rejects a
    // concurrent POST and gives loopback MCP calls an authoritative active run.
    this.runs.set(input.conversationId, record)
    this.deps.activity?.beginChatStream()
    record.activityStarted = true
    this.attachInitialPanels(record, input.panelTabIds ?? [])

    try {
      const stream = await input.createStream(
        record.abortController.signal,
        record.runId,
        (messages) =>
          this.updateMessages(record.conversationId, record.runId, messages),
      )
      record.reader = stream.getReader()
      // Stream factories publish canonical display messages before returning.
      // Release `/state` hydration only after that handoff is complete.
      this.markPrepared(record)
      if (record.abortController.signal.aborted) {
        await record.reader.cancel(record.abortController.signal.reason)
        await this.finish(record, 'aborted')
      } else {
        void this.pump(record)
      }
      return { runId: record.runId }
    } catch (error) {
      await this.finish(
        record,
        record.abortController.signal.aborted ? 'aborted' : 'failed',
      )
      throw error
    }
  }

  /** Replays buffered chunks, then attaches to the same live ordered stream. */
  subscribe(conversationId: string): ReadableStream<UIMessageChunk> {
    const record = this.runs.get(conversationId)
    if (!record) throw new ConversationRunNotFoundError()
    let subscriber: ReadableStreamDefaultController<UIMessageChunk> | undefined

    return new ReadableStream<UIMessageChunk>({
      start: (controller) => {
        subscriber = controller
        for (const chunk of record.chunks) controller.enqueue(chunk)
        if (record.status === 'running') {
          record.subscribers.add(controller)
        } else {
          controller.close()
        }
      },
      // An HTTP response or panel is only a view. Explicit `stop` is the sole
      // operation allowed to cancel the provider and its browser tool calls.
      cancel: () => {
        if (subscriber) record.subscribers.delete(subscriber)
      },
    })
  }

  /**
   * Streams the complete current assignment set immediately, on each change,
   * and periodically so a client can repair transient browser-side failures.
   */
  subscribePanelAssignments(): ReadableStream<ConversationPanelAssignments> {
    let subscriber: PanelAssignmentsSubscriber | undefined
    return new ReadableStream<ConversationPanelAssignments>({
      start: (controller) => {
        subscriber = { controller }
        this.panelSubscribers.add(subscriber)
        this.enqueuePanelAssignments(subscriber)
        subscriber.heartbeat = setInterval(() => {
          if (subscriber) this.enqueuePanelAssignments(subscriber)
        }, this.deps.panelAssignmentsHeartbeatMs ??
          DEFAULT_PANEL_ASSIGNMENTS_HEARTBEAT_MS)
      },
      cancel: () => {
        if (subscriber) this.removePanelAssignmentsSubscriber(subscriber)
      },
    })
  }

  /** Pins the exact live record so a late tool effect cannot mutate a later run. */
  activeRun(conversationId: string): ActiveConversationRun | undefined {
    const record = this.runs.get(conversationId)
    if (record?.status !== 'running' || record.abortController.signal.aborted) {
      return undefined
    }
    return {
      conversationId: record.conversationId,
      runId: record.runId,
      panelsVisible: record.panelsVisible,
      tabGroup: record.tabGroup,
      signal: record.abortController.signal,
      associateTabs: (tabIds) => this.associateTabs(record, tabIds),
    }
  }

  async stop(conversationId: string): Promise<boolean> {
    const record = this.runs.get(conversationId)
    if (record?.status !== 'running') return false

    const reason = new DOMException('Conversation stopped', 'AbortError')
    record.abortController.abort(reason)
    // Provider/MCP construction may still create resources after this call.
    // The stream factory owns unwinding them; finish only once it returns.
    if (!record.reader) return true

    await record.reader.cancel(reason).catch(() => undefined)
    await this.finish(record, 'aborted')
    return true
  }

  async delete(conversationId: string): Promise<boolean> {
    const record = this.runs.get(conversationId)
    if (!record) return false
    // Prevent a replacement turn until late stream construction has unwound.
    record.deleting = true
    if (record.status === 'running') await this.stop(conversationId)
    await record.finished
    if (this.runs.get(conversationId) !== record) return false

    this.runs.delete(conversationId)
    let panelsChanged = false
    for (const [tabId, owner] of this.panelByTab) {
      if (owner !== record) continue
      this.panelByTab.delete(tabId)
      panelsChanged = true
    }
    if (panelsChanged) this.publishPanelAssignments()
    return true
  }

  getSnapshot(conversationId: string): ConversationRunSnapshot | undefined {
    const record = this.runs.get(conversationId)
    if (!record) return undefined
    return this.snapshot(record)
  }

  /**
   * Waits only for provider/agent preparation, never for the full model run.
   * This closes the panel-open race where an immediate mapping precedes loaded
   * conversation history.
   */
  async getPreparedSnapshot(
    conversationId: string,
  ): Promise<ConversationRunSnapshot | undefined> {
    while (true) {
      const record = this.runs.get(conversationId)
      if (!record) return undefined
      await record.prepared
      if (this.runs.get(conversationId) === record) {
        return this.snapshot(record)
      }
    }
  }

  private snapshot(record: ConversationRunRecord): ConversationRunSnapshot {
    return {
      conversationId: record.conversationId,
      runId: record.runId,
      status: record.status,
      messages: [...record.messages],
      chunkCount: record.chunks.length,
    }
  }

  updateMessages(
    conversationId: string,
    runId: string,
    messages: UIMessage[],
  ): boolean {
    const record = this.runs.get(conversationId)
    if (!record || record.runId !== runId) return false
    record.messages = [...messages]
    return true
  }

  private attachInitialPanels(
    record: ConversationRunRecord,
    requestedTabIds: readonly number[],
  ): void {
    if (!record.panelsVisible) return
    const tabIds = new Set(requestedTabIds)
    // A later turn wakes every tab still mapped to this conversation. A tab
    // claimed by another conversation is absent unless explicitly requested.
    for (const [tabId, owner] of this.panelByTab) {
      if (owner.conversationId === record.conversationId) tabIds.add(tabId)
    }
    if (tabIds.size === 0) return
    for (const tabId of tabIds) this.panelByTab.set(tabId, record)
    this.publishPanelAssignments()
  }

  private associateTabs(
    record: ConversationRunRecord,
    tabIds: readonly number[],
  ): boolean {
    if (
      !record.panelsVisible ||
      record.status !== 'running' ||
      this.runs.get(record.conversationId) !== record
    ) {
      return false
    }

    let changed = false
    for (const tabId of tabIds) {
      if (!Number.isInteger(tabId) || tabId < 0) continue
      if (this.panelByTab.get(tabId) === record) continue
      this.panelByTab.set(tabId, record)
      changed = true
    }
    if (changed) this.publishPanelAssignments()
    return true
  }

  private currentPanelAssignments(): ConversationPanelAssignments {
    return {
      assignments: [...this.panelByTab.entries()]
        .map(([tabId, record]) => ({
          tabId,
          conversationId: record.conversationId,
          runId: record.runId,
          status: record.status,
        }))
        .sort((a, b) => a.tabId - b.tabId),
    }
  }

  private publishPanelAssignments(): void {
    const assignments = this.currentPanelAssignments()
    for (const subscriber of [...this.panelSubscribers]) {
      this.enqueuePanelAssignments(subscriber, assignments)
    }
  }

  private enqueuePanelAssignments(
    subscriber: PanelAssignmentsSubscriber,
    assignments = this.currentPanelAssignments(),
  ): void {
    try {
      subscriber.controller.enqueue(assignments)
    } catch {
      this.removePanelAssignmentsSubscriber(subscriber)
    }
  }

  private removePanelAssignmentsSubscriber(
    subscriber: PanelAssignmentsSubscriber,
  ): void {
    // The timer belongs to this HTTP stream. Clearing it on cancellation avoids
    // retaining a dead controller and the run graph behind its closure.
    if (subscriber.heartbeat) clearInterval(subscriber.heartbeat)
    this.panelSubscribers.delete(subscriber)
    subscriber.heartbeat = undefined
  }

  private async pump(record: ConversationRunRecord): Promise<void> {
    const reader = record.reader
    if (!reader) return
    try {
      while (true) {
        const { done, value } = await reader.read()
        if (done) break
        if (value.type === 'error') record.sawErrorChunk = true
        record.chunks.push(value)
        for (const subscriber of [...record.subscribers]) {
          try {
            subscriber.enqueue(value)
          } catch {
            record.subscribers.delete(subscriber)
          }
        }
      }
      await this.finish(
        record,
        record.abortController.signal.aborted
          ? 'aborted'
          : record.sawErrorChunk
            ? 'failed'
            : 'completed',
      )
    } catch {
      await this.finish(
        record,
        record.abortController.signal.aborted ? 'aborted' : 'failed',
      )
    } finally {
      reader.releaseLock()
    }
  }

  private async finish(
    record: ConversationRunRecord,
    status: Exclude<ConversationRunStatus, 'running'>,
  ): Promise<void> {
    if (record.ended) return
    record.ended = true
    record.status = status
    // Setup failures have no stream-return point, but panel hydration must still
    // resolve to the visible failed/aborted state instead of hanging forever.
    this.markPrepared(record)
    for (const subscriber of record.subscribers) {
      try {
        subscriber.close()
      } catch {
        // A detached HTTP response may already have closed its controller.
      }
    }
    record.subscribers.clear()

    if ([...this.panelByTab.values()].includes(record)) {
      this.publishPanelAssignments()
    }
    try {
      if (record.activityStarted) this.deps.activity?.endChatStream()
    } finally {
      record.resolveFinished()
    }
  }

  private markPrepared(record: ConversationRunRecord): void {
    if (record.preparedResolved) return
    record.preparedResolved = true
    record.resolvePrepared()
  }
}
