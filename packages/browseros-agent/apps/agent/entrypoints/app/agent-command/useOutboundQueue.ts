import { useCallback, useEffect, useRef, useState } from 'react'
import type { UserAttachmentPreview } from '@/lib/agent-conversations/types'
import type { ServerAttachmentPayload } from '@/lib/attachments'
import type { SendInput } from './useAgentConversation'

export type OutboundMessageStatus = 'queued' | 'sending' | 'failed'

export interface OutboundMessage {
  id: string
  text: string
  attachments: ServerAttachmentPayload[]
  attachmentPreviews: UserAttachmentPreview[]
  status: OutboundMessageStatus
  error?: string
  createdAt: number
}

export interface OutboundQueueEnqueueInput {
  text: string
  attachments?: ServerAttachmentPayload[]
  attachmentPreviews?: UserAttachmentPreview[]
}

export interface OutboundQueueApi {
  queue: OutboundMessage[]
  enqueue(input: OutboundQueueEnqueueInput): void
  cancel(id: string): void
  retry(id: string): void
}

interface UseOutboundQueueOptions {
  send: (input: SendInput) => Promise<void>
  streaming: boolean
}

function makeId(): string {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID()
  }
  return `outbound-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
}

/**
 * Client-side outbound queue for agent chat sends. Allows the user to keep
 * typing while the agent is mid-turn — each enqueued message is delivered
 * as soon as `streaming` flips false (i.e., the prior turn finished).
 *
 * The hook owns no scheduling timers; it reacts to `streaming` transitions
 * and to enqueue calls. The worker is single-flight by design — only one
 * message is dispatched at a time, in arrival order.
 */
export function useOutboundQueue(
  options: UseOutboundQueueOptions,
): OutboundQueueApi {
  const { send, streaming } = options
  const [queue, setQueue] = useState<OutboundMessage[]>([])
  const sendRef = useRef(send)
  sendRef.current = send

  // Re-entrancy guard: while a queued item is mid-dispatch we don't want
  // a streaming flip to kick off a second simultaneous dispatch.
  const dispatchingRef = useRef(false)

  const setStatus = useCallback(
    (id: string, status: OutboundMessageStatus, error?: string) => {
      setQueue((prev) =>
        prev.map((m) =>
          m.id === id
            ? { ...m, status, error: status === 'failed' ? error : undefined }
            : m,
        ),
      )
    },
    [],
  )

  const removeItem = useCallback((id: string) => {
    setQueue((prev) => prev.filter((m) => m.id !== id))
  }, [])

  const enqueue = useCallback((input: OutboundQueueEnqueueInput) => {
    const trimmed = input.text.trim()
    const attachments = input.attachments ?? []
    if (!trimmed && attachments.length === 0) return
    const message: OutboundMessage = {
      id: makeId(),
      text: trimmed,
      attachments,
      attachmentPreviews: input.attachmentPreviews ?? [],
      status: 'queued',
      createdAt: Date.now(),
    }
    setQueue((prev) => [...prev, message])
  }, [])

  const cancel = useCallback((id: string) => {
    setQueue((prev) => {
      const target = prev.find((m) => m.id === id)
      // Only allow cancelling items that haven't started sending yet.
      // Sending items are owned by the SSE stream and need a separate
      // abort path (deferred to v2).
      if (!target || target.status === 'sending') return prev
      return prev.filter((m) => m.id !== id)
    })
  }, [])

  const retry = useCallback((id: string) => {
    setQueue((prev) =>
      prev.map((m) =>
        m.id === id && m.status === 'failed'
          ? { ...m, status: 'queued', error: undefined }
          : m,
      ),
    )
  }, [])

  // Drain the queue whenever `streaming` is false and we have a head item
  // in `queued`. The effect runs on every queue/streaming change but the
  // dispatching ref keeps it single-flight.
  useEffect(() => {
    if (streaming) return
    if (dispatchingRef.current) return
    const head = queue.find((m) => m.status === 'queued')
    if (!head) return

    let cancelled = false
    dispatchingRef.current = true
    setStatus(head.id, 'sending')
    ;(async () => {
      try {
        await sendRef.current({
          text: head.text,
          attachments: head.attachments,
          attachmentPreviews: head.attachmentPreviews,
        })
        if (!cancelled) removeItem(head.id)
      } catch (err) {
        if (!cancelled) {
          const message =
            err instanceof Error ? err.message : 'Failed to send message'
          setStatus(head.id, 'failed', message)
        }
      } finally {
        dispatchingRef.current = false
      }
    })()

    return () => {
      cancelled = true
    }
  }, [queue, streaming, removeItem, setStatus])

  return { queue, enqueue, cancel, retry }
}
