import type { Chat } from '@ai-sdk/react'
import type { UIMessage } from 'ai'
import { fetchConversationRunState } from './conversation-run-client'

/**
 * One view's cancellable, exact-run subscriber. The SDK reducer consumes replay
 * from the immutable prepared history on every retry, so partial answers are
 * neither appended twice nor mixed with a successor turn. Detach never calls Stop.
 */
export async function attachConversationRun(input: {
  chat: Chat<UIMessage>
  conversationId: string
  runId: string
  serverUrl: string
  signal: AbortSignal
  fetch?: typeof fetch
  retryMs?: number
  onHydrated?: () => void
  onError?: (error: unknown) => void
}): Promise<void> {
  const { chat, conversationId, runId, serverUrl, signal } = input
  let consumingReplay = false
  const detach = () => {
    // After replay ends the SDK is available for another POST, even while our
    // final state read is pending. Cleanup owns only this replay's consumption.
    if (consumingReplay) void chat.stop()
  }
  signal.addEventListener('abort', detach, { once: true })
  try {
    while (!signal.aborted) {
      try {
        const state = await fetchConversationRunState(
          serverUrl,
          conversationId,
          input.fetch,
          { runId, signal },
        )
        if (signal.aborted) return
        await chat.stop()
        if (signal.aborted) return
        chat.messages = state.replayMessages ?? state.messages
        input.onHydrated?.()
        consumingReplay = true
        try {
          await chat.resumeStream({ body: { runId } })
        } finally {
          consumingReplay = false
        }
        if (signal.aborted) return
        // SDK reconnect errors normally resolve and set chat.error. A broken
        // stream must remain retryable even when the assignment never changes.
        const after = await fetchConversationRunState(
          serverUrl,
          conversationId,
          input.fetch,
          { runId, signal },
        )
        if (signal.aborted) return
        if (after.status === 'completed' && chat.status === 'error') {
          // Completion makes the server's final messages authoritative. A
          // failed replay fetch must not leave a finished turn stuck at its
          // prepared history, nor show a transport error as a model failure.
          chat.messages = after.messages
          chat.clearError()
        }
        if (after.status !== 'running') return
        if (chat.status === 'error') throw chat.error
      } catch (error) {
        if (signal.aborted) return
        input.onError?.(error)
      }
      await new Promise<void>((resolve) => {
        const done = () => {
          clearTimeout(timer)
          signal.removeEventListener('abort', done)
          resolve()
        }
        const timer = setTimeout(done, input.retryMs ?? 1000)
        signal.addEventListener('abort', done, { once: true })
        if (signal.aborted) done()
      })
    }
  } finally {
    signal.removeEventListener('abort', detach)
  }
}
