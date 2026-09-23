export type ChatSendDecision = 'send' | 'queue' | 'refuse' | 'drop'

export interface ChatSendDecisionInput {
  /** The conversation is being restored, so a send now would race it. */
  isRestoring: boolean
  hasRestoreError: boolean
  /** Whether the provider and agent lists have finished loading. */
  isSettled: boolean
  hasAnyTarget: boolean
  isIntegrationsSynced: boolean
  hasAgentUrl: boolean
}

/**
 * What to do with a send.
 *
 * `refuse` is answered on screen, `queue` is retained and flushed later, and
 * `drop` is neither because the caller is mid-restore and will try again.
 *
 * The order matters more than any single branch: absence of a target only
 * means "nothing connected" once the lists have settled. Refusing before that
 * discards the handoff from the home composer, which arrives on mount with its
 * query parameters already consumed, so there is nothing left to retry from.
 */
export function decideChatSend(input: ChatSendDecisionInput): ChatSendDecision {
  if (input.isRestoring || input.hasRestoreError) return 'drop'
  if (input.isSettled && !input.hasAnyTarget) return 'refuse'
  if (!input.isSettled || !input.isIntegrationsSynced || !input.hasAgentUrl) {
    return 'queue'
  }
  return 'send'
}

/**
 * Sends everything held during a wait, in the order it was written.
 *
 * Sequential, not concurrent. These are consecutive turns in one conversation,
 * so starting the next before the previous finishes would put two streams on
 * the same conversation at once.
 */
export async function drainPendingSends<T>(
  queued: readonly T[],
  send: (item: T) => Promise<unknown>,
): Promise<void> {
  for (const item of queued) {
    await send(item)
  }
}
