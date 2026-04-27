import { useCallback, useEffect, useRef, useState } from 'react'
import type { OpenClawChatHistoryMessage } from '@/entrypoints/app/agents/useOpenClaw'
import type { UserAttachmentPreview } from '@/lib/agent-conversations/types'
import type { ServerAttachmentPayload } from '@/lib/attachments'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

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
  history?: OpenClawChatHistoryMessage[]
}

export interface OutboundQueueApi {
  queue: OutboundMessage[]
  enqueue(input: OutboundQueueEnqueueInput): void
  cancel(id: string): void
  retry(id: string): void
}

interface UseOutboundQueueOptions {
  agentId: string | null | undefined
  sessionKey?: string | null
}

interface ServerQueuedItem {
  id: string
  status: 'queued' | 'dispatching' | 'failed'
  message: string
  attachmentsPreview: Array<{
    kind: 'image' | 'file'
    mediaType: string
    name?: string
  }>
  error?: string
  createdAt: number
}

function makeId(): string {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return crypto.randomUUID()
  }
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
}

/**
 * Server-backed outbound message queue. The browser is purely a
 * projection of server state — closing the tab is safe because the queue
 * keeps draining server-side via the OutboundQueueService.
 *
 * Single id-keyed list: the client generates the queue id and hands it
 * to the server in the POST body, so the optimistic row and the SSE
 * snapshot reconcile on the same key from frame zero — there is no
 * window in which the message renders twice.
 */
export function useOutboundQueue(
  options: UseOutboundQueueOptions,
): OutboundQueueApi {
  const { agentId, sessionKey } = options
  const { baseUrl } = useAgentServerUrl()
  const sessionKeyRef = useRef<string | null | undefined>(sessionKey)
  sessionKeyRef.current = sessionKey

  const [items, setItems] = useState<OutboundMessage[]>([])
  // Track which ids the server has confirmed seeing in any SSE snapshot.
  // We use this to know whether a missing-from-snapshot id is "drained
  // by the server" (drop it) or "still in flight client-side" (keep
  // showing the optimistic row).
  const everSeenByServerRef = useRef<Set<string>>(new Set())
  // Local-only attachment previews, keyed by queue id. Data URLs never
  // leave the browser — the SSE feed only carries metadata, so we hold
  // them here so the chip strip keeps rendering after server takeover.
  const previewMapRef = useRef<Map<string, UserAttachmentPreview[]>>(new Map())

  useEffect(() => {
    if (!baseUrl || !agentId) {
      setItems([])
      everSeenByServerRef.current = new Set()
      previewMapRef.current = new Map()
      return
    }
    let cancelled = false
    const url = `${baseUrl}/claw/agents/${encodeURIComponent(agentId)}/queue/stream`
    const source = new EventSource(url)
    source.onmessage = (event) => {
      if (cancelled) return
      try {
        const parsed = JSON.parse(event.data) as { items: ServerQueuedItem[] }
        const snapshotIds = new Set(parsed.items.map((item) => item.id))
        for (const id of snapshotIds) everSeenByServerRef.current.add(id)

        setItems((prev) => {
          const next: OutboundMessage[] = parsed.items.map((item) => ({
            id: item.id,
            text: item.message,
            attachments: [],
            attachmentPreviews: previewMapRef.current.get(item.id) ?? [],
            status: serverStatusToClient(item.status),
            error: item.error,
            createdAt: item.createdAt,
          }))
          // Carry forward any optimistic / failed entries the server
          // doesn't know about yet (POST in flight) or has finished
          // dispatching but the client wants to keep visible (failed).
          const carried = prev.filter((local) => {
            if (snapshotIds.has(local.id)) return false
            if (everSeenByServerRef.current.has(local.id)) {
              // Server saw it before and it's gone now — drained.
              previewMapRef.current.delete(local.id)
              return false
            }
            return local.status !== 'failed' || Boolean(local.error)
          })
          return [...carried, ...next]
        })
      } catch {
        // Malformed event — ignore; next snapshot will recover.
      }
    }
    source.onerror = () => {
      // Auto-reconnects; nothing to do here.
    }
    return () => {
      cancelled = true
      source.close()
    }
  }, [baseUrl, agentId])

  const enqueue = useCallback(
    (input: OutboundQueueEnqueueInput) => {
      if (!baseUrl || !agentId) return
      const trimmed = input.text.trim()
      const attachments = input.attachments ?? []
      if (!trimmed && attachments.length === 0) return

      const id = makeId()
      const previews = input.attachmentPreviews ?? []
      previewMapRef.current.set(id, previews)
      setItems((prev) => [
        ...prev,
        {
          id,
          text: trimmed,
          attachments,
          attachmentPreviews: previews,
          status: 'queued',
          createdAt: Date.now(),
        },
      ])

      void (async () => {
        try {
          const response = await fetch(
            `${baseUrl}/claw/agents/${encodeURIComponent(agentId)}/queue`,
            {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({
                id,
                message: trimmed,
                attachments: attachments.length > 0 ? attachments : undefined,
                sessionKey: sessionKeyRef.current ?? undefined,
                history: input.history,
              }),
            },
          )
          if (!response.ok) {
            const text = await response.text().catch(() => '')
            previewMapRef.current.delete(id)
            setItems((prev) =>
              prev.map((item) =>
                item.id === id
                  ? {
                      ...item,
                      status: 'failed',
                      error:
                        text || `Failed to enqueue (status ${response.status})`,
                    }
                  : item,
              ),
            )
          }
        } catch (err) {
          // Only mark as failed if the SSE snapshot hasn't already
          // taken ownership of the entry (i.e. the request actually
          // reached the server).
          if (everSeenByServerRef.current.has(id)) return
          previewMapRef.current.delete(id)
          setItems((prev) =>
            prev.map((item) =>
              item.id === id
                ? {
                    ...item,
                    status: 'failed',
                    error:
                      err instanceof Error
                        ? err.message
                        : 'Failed to enqueue message',
                  }
                : item,
            ),
          )
        }
      })()
    },
    [baseUrl, agentId],
  )

  const cancel = useCallback(
    (id: string) => {
      // If the server has never seen this id, just drop it locally.
      if (!everSeenByServerRef.current.has(id)) {
        previewMapRef.current.delete(id)
        setItems((prev) => prev.filter((item) => item.id !== id))
        return
      }
      if (!baseUrl || !agentId) return
      void fetch(
        `${baseUrl}/claw/agents/${encodeURIComponent(agentId)}/queue/${encodeURIComponent(id)}`,
        { method: 'DELETE' },
      ).catch(() => {})
    },
    [baseUrl, agentId],
  )

  const retry = useCallback(
    (id: string) => {
      if (!everSeenByServerRef.current.has(id)) {
        // Optimistic-only entry, never made it to the server. Reset
        // status so the user can press Send again.
        setItems((prev) =>
          prev.map((item) =>
            item.id === id
              ? { ...item, status: 'queued', error: undefined }
              : item,
          ),
        )
        return
      }
      if (!baseUrl || !agentId) return
      void fetch(
        `${baseUrl}/claw/agents/${encodeURIComponent(agentId)}/queue/${encodeURIComponent(id)}/retry`,
        { method: 'POST' },
      ).catch(() => {})
    },
    [baseUrl, agentId],
  )

  return { queue: items, enqueue, cancel, retry }
}

function serverStatusToClient(
  status: ServerQueuedItem['status'],
): OutboundMessageStatus {
  if (status === 'dispatching') return 'sending'
  if (status === 'failed') return 'failed'
  return 'queued'
}
