import { useCallback, useEffect, useRef, useState } from 'react'
import type { OpenClawChatHistoryMessage } from '@/entrypoints/app/agents/useOpenClaw'
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
  /**
   * Prior chat turns to forward verbatim. Mirrors the direct chat path so
   * queued sends never lose conversational context — the queue worker
   * passes this through to OpenClaw as the message history.
   */
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
  /**
   * Current resolved sessionKey for the active conversation. The hook
   * forwards it to the server on every enqueue so the queue worker
   * targets the same OpenClaw session the user is actively viewing.
   * Null means "no session yet" — the server will allocate one.
   */
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
  const { agentId, sessionKey } = options
  const { baseUrl } = useAgentServerUrl()
  // Keep the latest sessionKey in a ref so enqueue's closure always sees
  // the freshest value without re-creating the callback on every change.
  const sessionKeyRef = useRef<string | null | undefined>(sessionKey)
  sessionKeyRef.current = sessionKey
  const [serverItems, setServerItems] = useState<OutboundMessage[]>([])
  const [localItems, setLocalItems] = useState<OutboundMessage[]>([])

  // Map local previews onto server-issued ids. The POST response gives us
  // the server id, and we keep a localId fallback for the brief window
  // before that response lands. Keying by id (never by message text)
  // avoids collisions when the user sends two messages with identical
  // text back-to-back.
  const previewMapRef = useRef<
    Map<string, { localId: string; previews: UserAttachmentPreview[] }>
  >(new Map())

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
          const matched = previewMapRef.current.get(item.id)
          return {
            id: item.id,
            text: item.message,
            attachments: [],
            attachmentPreviews: matched?.previews ?? [],
            status: serverStatusToClient(item.status),
            error: item.error,
            createdAt: item.createdAt,
          }
        })
        setServerItems(next)
        // Drop optimistic entries that the server has now acknowledged.
        // We tracked their localId in the preview map alongside the
        // server id, so when an item's server id appears in the snapshot
        // its sibling localId is safe to evict.
        const acknowledgedLocalIds = new Set<string>()
        for (const item of next) {
          const matched = previewMapRef.current.get(item.id)
          if (matched) acknowledgedLocalIds.add(matched.localId)
        }
        setLocalItems((prev) =>
          prev.filter((local) => !acknowledgedLocalIds.has(local.id)),
        )
        // Prune preview entries whose server item is no longer in the
        // snapshot — once the queue drains it we can free the data URLs.
        const liveIds = new Set(next.map((item) => item.id))
        for (const id of previewMapRef.current.keys()) {
          if (id.startsWith(LOCAL_PREFIX)) continue
          if (!liveIds.has(id)) previewMapRef.current.delete(id)
        }
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
      // Stage previews under the localId until the server responds with
      // the real id. Once we have that id we re-key under it so SSE
      // snapshots can match.
      if (previews.length > 0) {
        previewMapRef.current.set(localId, { localId, previews })
      }

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
                sessionKey: sessionKeyRef.current ?? undefined,
                history: input.history,
              }),
            },
          )
          if (!response.ok) {
            const text = await response.text().catch(() => '')
            previewMapRef.current.delete(localId)
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
            return
          }
          const payload = (await response.json().catch(() => null)) as {
            id?: string
          } | null
          const serverId = payload?.id
          if (serverId && previews.length > 0) {
            previewMapRef.current.set(serverId, { localId, previews })
            previewMapRef.current.delete(localId)
          }
        } catch (err) {
          previewMapRef.current.delete(localId)
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
        previewMapRef.current.delete(id)
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
