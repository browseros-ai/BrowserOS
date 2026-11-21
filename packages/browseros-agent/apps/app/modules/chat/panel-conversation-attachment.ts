import type { ConversationPanelAssignment } from '@browseros/shared/schemas/conversation-panels'
import type { UIMessage } from 'ai'
import type { ConversationRunState } from './conversation-run-client'

/**
 * What the panel follows, and what a retry hydrates again: the run the broker assigned to this
 * panel's tab, or a conversation the user opened from History. A History selection belongs to no
 * tab binding and may have no run, so it carries its stored transcript for that case.
 */
type Subject = { conversationId: string } & (
  | { kind: 'assigned'; runId: string }
  | { kind: 'opened'; stored: UIMessage[] }
)

/**
 * What the panel installs. `liveRunId` is set only while the server is still running the turn:
 * `messages` then stops at the user's message, and the panel replays that run's stream from its
 * first chunk and follows it live, exactly as the server's `/state` and `/stream` pair promises.
 * `reloaded` is a re-read after this panel's Stop, which follows nothing new.
 */
export interface PanelSnapshot {
  source: Subject['kind'] | 'reloaded'
  conversationId: string
  messages: UIMessage[]
  liveRunId?: string
}

export interface PanelConversationAttachmentDeps {
  /** The server's run for the conversation, or `undefined` when it knows none. */
  load(
    conversationId: string,
    signal: AbortSignal,
  ): Promise<ConversationRunState | undefined>
  /** The stored transcript, or `undefined` when none was stored (an incognito turn) or it is gone. */
  loadStored(conversationId: string): Promise<UIMessage[] | undefined>
  attach(snapshot: PanelSnapshot, isCurrent: () => boolean): Promise<void>
  clear(conversationId: string): void
  ownsLocalStream(conversationId: string, runId: string): boolean
  reportError(error: unknown): void
  retryMs?: number
}

/**
 * One panel's subscription lifecycle. It follows one subject at a time, whichever came last: a
 * new run the broker assigns to the tab, or a History selection. Epochs cancel stale hydration
 * across new turns, reassignment, History selections, and retirement; explicit retries also
 * cover SDK failures reported through onError instead of rejected resumeStream promises.
 */
export class PanelConversationAttachment {
  /** The broker's latest binding for this tab. Heartbeats repeat it and must not re-hydrate. */
  private assignment: ConversationPanelAssignment | undefined
  private subject: Subject | undefined
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
    const previous = this.assignment
    this.assignment = view
    if (key(view) === key(previous)) return
    if (!view) {
      // The tab losing its binding says nothing about a conversation opened from History.
      if (this.subject?.kind === 'opened') return
      this.follow(undefined)
      if (previous) this.deps.clear(previous.conversationId)
      return
    }
    this.follow({
      kind: 'assigned',
      conversationId: view.conversationId,
      runId: view.runId,
    })
    if (this.deps.ownsLocalStream(view.conversationId, view.runId)) return
    void this.hydrate()
  }

  /**
   * Shows a conversation the user opened from History. The caller reads `run` after `stored`, so
   * a run that ends in between still answers with its whole transcript: a live run is followed to
   * its end, a finished one shows its whole transcript, and with no run the stored one stands.
   *
   * `isCancelled` is the restore's own guard. A newer History selection cancels this one before
   * its requests reach `open`, so the install checks it too, after attach has detached the view.
   */
  async open(
    conversationId: string,
    stored: UIMessage[],
    run: ConversationRunState | undefined,
    isCancelled: () => boolean,
  ): Promise<void> {
    if (this.disposed || isCancelled()) return
    // An explicit history selection may resume a previously retired conversation.
    this.retired.delete(conversationId)
    const subject: Subject = { kind: 'opened', conversationId, stored }
    this.follow(subject)
    const isCurrent = this.nextEpoch()
    await this.deps.attach(
      snapshotOf(subject, run),
      () => isCurrent() && !isCancelled(),
    )
    // A selection cancelled before it landed must not come back on a retry.
    if (isCancelled() && this.subject === subject) this.follow(undefined)
  }

  /**
   * Shows what the server recorded for a run this panel stopped. Stop detaches the view before it
   * asks the server to cancel, so the run's last chunks (each stopped tool part, then `abort`)
   * never reach this panel. Once `stopping` settles, the run's state carries them: whole once the
   * run has ended, or replayed to its end while it still unwinds. A run the server no longer
   * retains leaves the stored transcript as the truth; with neither, the panel keeps what it shows.
   *
   * Call it when Stop is pressed. It takes the epoch current then and starts none, because it
   * follows nothing new: anything that starts afterwards (a local turn, a History selection, New
   * conversation, a new run for the tab) discards it, and it never discards them. Installing past
   * a newer local turn would detach that turn's stream. `isShown` is the hook's guard against a
   * panel that already moved to another conversation through a selection still in flight.
   */
  async reload(
    conversationId: string,
    stopping: Promise<unknown>,
    isShown: () => boolean,
  ): Promise<void> {
    const epoch = this.epoch
    const isCurrent = () => !this.disposed && epoch === this.epoch && isShown()
    try {
      await stopping
      if (!isCurrent()) return
      const run = await this.deps.load(
        conversationId,
        new AbortController().signal,
      )
      if (!isCurrent()) return
      const messages =
        run?.messages ?? (await this.deps.loadStored(conversationId))
      if (!messages || !isCurrent()) return
      await this.deps.attach(
        {
          source: 'reloaded',
          conversationId,
          messages,
          liveRunId: run?.status === 'running' ? run.runId : undefined,
        },
        isCurrent,
      )
    } catch (error) {
      if (isCurrent()) this.deps.reportError(error)
    }
  }

  retire(conversationId: string): void {
    this.retired.add(conversationId)
    // The panel moves to a new conversation, so a retry must not bring back what it followed.
    this.follow(undefined)
    if (this.assignment?.conversationId === conversationId)
      this.assignment = undefined
  }

  beginLocalTurn(conversationId: string): void {
    // An explicit history selection may resume a previously retired conversation.
    this.retired.delete(conversationId)
    this.invalidate()
  }

  retry(): void {
    const subject = this.subject
    if (this.disposed || !subject || this.retired.has(subject.conversationId))
      return
    this.invalidate()
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

  private follow(subject: Subject | undefined): void {
    this.invalidate()
    this.subject = subject
    this.retryDelay = 0
  }

  private invalidate(): void {
    this.epoch += 1
    this.abort?.abort()
    clearTimeout(this.retryTimer)
  }

  /** Starts an epoch. Its check turns false as soon as anything newer starts. */
  private nextEpoch(): () => boolean {
    const epoch = ++this.epoch
    return () => !this.disposed && epoch === this.epoch
  }

  private async hydrate(): Promise<void> {
    const subject = this.subject
    if (!subject || this.disposed) return
    const isCurrent = this.nextEpoch()
    this.abort = new AbortController()
    try {
      const run = await this.deps.load(
        subject.conversationId,
        this.abort.signal,
      )
      if (!isCurrent()) return
      await this.deps.attach(snapshotOf(subject, run), isCurrent)
    } catch (error) {
      if (!isCurrent()) return
      this.deps.reportError(error)
      this.retry()
    }
  }
}

/** A broker assignment needs its exact run; a History selection takes whatever run the server has. */
function snapshotOf(
  subject: Subject,
  run: ConversationRunState | undefined,
): PanelSnapshot {
  const { kind: source, conversationId } = subject
  if (!run) {
    if (subject.kind === 'assigned')
      throw new Error('The server knows no run for this panel assignment')
    return { source, conversationId, messages: subject.stored }
  }
  if (
    run.conversationId !== conversationId ||
    (subject.kind === 'assigned' && run.runId !== subject.runId)
  ) {
    throw new Error('Panel assignment changed during hydration')
  }
  return {
    source,
    conversationId,
    messages: run.messages,
    liveRunId: run.status === 'running' ? run.runId : undefined,
  }
}

function key(view: ConversationPanelAssignment | undefined): string {
  return view ? `${view.conversationId}:${view.runId}` : ''
}
