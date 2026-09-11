import type { ConversationPanelAssignment } from '@browseros/shared/schemas/conversation-panels'
import type { ConversationRunState } from './conversation-run-client'

export interface PanelConversationAttachmentDeps {
  load(
    conversationId: string,
    signal: AbortSignal,
  ): Promise<ConversationRunState>
  attach(state: ConversationRunState, isCurrent: () => boolean): Promise<void>
  clear(conversationId: string): void
  ownsLocalStream(conversationId: string, runId: string): boolean
  reportError(error: unknown): void
  retryMs?: number
}

/**
 * One tab's subscription lifecycle. Epochs cancel stale hydration across new
 * turns, reassignment, and retirement; explicit retries also cover SDK failures
 * reported through onError instead of rejected resumeStream promises.
 */
export class PanelConversationAttachment {
  private view: ConversationPanelAssignment | undefined
  private attached = ''
  private epoch = 0
  private abort: AbortController | undefined
  private retryTimer: ReturnType<typeof setTimeout> | undefined
  private readonly retired = new Set<string>()
  private disposed = false
  private retryDelay = 0

  constructor(private readonly deps: PanelConversationAttachmentDeps) {}

  update(view: ConversationPanelAssignment | undefined): void {
    if (this.disposed) return
    if (view && this.retired.has(view.conversationId)) return
    if (key(view) === key(this.view)) {
      this.view = view
      return
    }
    const previous = this.view
    this.invalidate()
    this.view = view
    this.attached = ''
    this.retryDelay = 0
    if (!view) {
      if (previous) this.deps.clear(previous.conversationId)
      return
    }
    if (this.deps.ownsLocalStream(view.conversationId, view.runId)) {
      this.attached = key(view)
      return
    }
    void this.hydrate()
  }

  retire(conversationId: string): void {
    this.retired.add(conversationId)
    this.invalidate()
    if (this.view?.conversationId === conversationId) {
      this.view = undefined
      this.attached = ''
    }
  }

  beginLocalTurn(conversationId: string): void {
    // An explicit history selection may resume a previously retired conversation.
    this.retired.delete(conversationId)
    this.invalidate()
  }

  retry(): void {
    if (
      this.disposed ||
      !this.view ||
      this.retired.has(this.view.conversationId)
    )
      return
    this.invalidate()
    this.attached = ''
    this.retryDelay = Math.min(
      this.retryDelay ? this.retryDelay * 2 : (this.deps.retryMs ?? 250),
      5_000,
    )
    this.retryTimer = setTimeout(() => void this.hydrate(), this.retryDelay)
  }

  dispose(): void {
    this.disposed = true
    this.invalidate()
  }

  private invalidate(): void {
    this.epoch += 1
    this.abort?.abort()
    clearTimeout(this.retryTimer)
  }

  private async hydrate(): Promise<void> {
    const view = this.view
    if (!view || this.disposed || this.attached === key(view)) return
    const epoch = ++this.epoch
    this.abort = new AbortController()
    const isCurrent = () => !this.disposed && epoch === this.epoch
    try {
      const state = await this.deps.load(view.conversationId, this.abort.signal)
      if (!isCurrent()) return
      if (
        state.conversationId !== view.conversationId ||
        state.runId !== view.runId
      ) {
        throw new Error('Panel assignment changed during hydration')
      }
      this.attached = key(view)
      await this.deps.attach(state, isCurrent)
    } catch (error) {
      if (!isCurrent()) return
      this.deps.reportError(error)
      this.retry()
    }
  }
}

function key(view: ConversationPanelAssignment | undefined): string {
  return view ? `${view.conversationId}:${view.runId}` : ''
}
