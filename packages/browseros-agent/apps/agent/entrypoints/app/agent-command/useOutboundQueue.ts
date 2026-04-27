import { useCallback, useEffect, useRef, useState } from 'react'
import type { UserAttachmentPreview } from '@/lib/agent-conversations/types'
import type { ServerAttachmentPayload } from '@/lib/attachments'
import { useAgentServerUrl } from '@/lib/browseros/useBrowserOSProviders'

export type OutboundMessageStatus = 'queued' | 'sending' | 'failed'

/**
 * The composer renders this shape. It mirrors the server's QueuedItemPublic
 * but adds local-only `attachmentPreviews` (data URLs) so chip thumbnails
 * keep rendering while a message is in flight to / on the server. Those
 * data URLs never leave the browser; the SSE feed only carries metadata.
 */
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
  agentId: string | null | undefined
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

const LOCAL_PREFIX = 'local-'

function makeLocalId(): string {
  if (typeof crypto !== 'undefined' && crypto.randomUUID) {
    return `${LOCAL_PREFIX}${crypto.randomUUID()}`
  }
  return `${LOCAL_PREFIX}${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 10)}`
}

/**
 * Server-backed outbound message queue. The browser is purely a
 * projection of server state — closing the tab is safe because the queue
 * keeps draining server-side via the OutboundQueueService.
 *
 * The hook still exposes the same `{ queue, enqueue, cancel, retry }`
 * surface the composer expects. `enqueue` POSTs to /queue and shows an
 * optimistic local entry until the server's SSE snapshot reflects it
 * (then the optimistic entry is replaced).
 */
export function useOutboundQueue(
  options: UseOutboundQueueOptions,
): OutboundQueueApi {
  const { agentId } = options
  const { baseUrl } = useAgentServerUrl()
  const [serverItems, setServerItems] = useState<OutboundMessage[]>([])
  const [localItems, setLocalItems] = useState<OutboundMessage[]>([])

  // Keep the latest local previews keyed by the local id so when the
  // server snapshot lands we can carry the data URLs forward onto the
  // server-issued id (matched by message text + arrival order — best
  // effort, fine for the chip strip).
  const localPreviewsRef = useRef<
    Array<{ text: string; previews: UserAttachmentPreview[] }>
  >([])

  // Subscribe to per-agent queue stream.
  useEffect(() => {
    if (!baseUrl || !agentId) {
      setServerItems([])
      return
    }
    let cancelled = false
    const url = `${baseUrl}/claw/agents/${encodeURIComponent(agentId)}/queue/stream`
    const source = new EventSource(url)
    source.onmessage = (event) => {
      if (cancelled) return
      try {
        const parsed = JSON.parse(event.data) as { items: ServerQueuedItem[] }
        const next: OutboundMessage[] = parsed.items.map((item) => {
          const matchedPreview = localPreviewsRef.current.find(
            (entry) => entry.text === item.message,
          )
          return {
            id: item.id,
            text: item.message,
            attachments: [],
            attachmentPreviews: matchedPreview?.previews ?? [],
            status: serverStatusToClient(item.status),
            error: item.error,
            createdAt: item.createdAt,
          }
        })
        setServerItems(next)
        // Drop any optimistic entries whose text now appears in the
        // server snapshot — the server has acknowledged them.
        setLocalItems((prev) =>
          prev.filter((local) => !next.some((s) => s.text === local.text)),
        )
      } catch {
        // Malformed event — ignore; next snapshot will recover.
      }
    }
    source.onerror = () => {
      // Connection issues are transient (server restart, network blip).
      // EventSource auto-reconnects; nothing to do here.
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

      const localId = makeLocalId()
      const previews = input.attachmentPreviews ?? []
      const optimistic: OutboundMessage = {
        id: localId,
        text: trimmed,
        attachments,
        attachmentPreviews: previews,
        status: 'queued',
        createdAt: Date.now(),
      }
      setLocalItems((prev) => [...prev, optimistic])
      // Remember the previews so they can be re-attached when the server
      // snapshot replaces the optimistic entry.
      localPreviewsRef.current = [
        ...localPreviewsRef.current,
        { text: trimmed, previews },
      ]

      void (async () => {
        try {
          const response = await fetch(
            `${baseUrl}/claw/agents/${encodeURIComponent(agentId)}/queue`,
            {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({
                message: trimmed,
                attachments: attachments.length > 0 ? attachments : undefined,
              }),
            },
          )
          if (!response.ok) {
            const text = await response.text().catch(() => '')
            setLocalItems((prev) =>
              prev.map((item) =>
                item.id === localId
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
          setLocalItems((prev) =>
            prev.map((item) =>
              item.id === localId
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
      if (id.startsWith(LOCAL_PREFIX)) {
        setLocalItems((prev) => prev.filter((item) => item.id !== id))
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
      if (id.startsWith(LOCAL_PREFIX)) {
        // Optimistic local items don't have a server id yet — reset them
        // to 'queued' so the user can press Send again. Caller is
        // expected to re-enqueue manually for v1.
        setLocalItems((prev) =>
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

  // Order: local optimistic entries first (just-pressed), then server
  // entries. When the SSE snapshot arrives, the local entry for the same
  // text has already been pruned above, so this never duplicates.
  const queue = [...localItems, ...serverItems]

  return { queue, enqueue, cancel, retry }
}

function serverStatusToClient(
  status: ServerQueuedItem['status'],
): OutboundMessageStatus {
  if (status === 'dispatching') return 'sending'
  if (status === 'failed') return 'failed'
  return 'queued'
}
