import { useEffect, useRef, useState } from 'react'
import {
  chatWithAgent,
  type OpenClawChatHistoryMessage,
  type OpenClawStreamEvent,
} from '@/entrypoints/app/agents/useOpenClaw'
import type {
  AgentConversationTurn,
  AssistantPart,
  UserAttachmentPreview,
} from '@/lib/agent-conversations/types'
import type { ServerAttachmentPayload } from '@/lib/attachments'
import { consumeSSEStream } from '@/lib/sse'
import { buildToolLabel } from '@/lib/tool-labels'

export interface SendInput {
  text: string
  attachments?: ServerAttachmentPayload[]
  // Optional preview metadata used to render the optimistic user turn.
  // Built by the composer at staging time; the server only sees the
  // payload array.
  attachmentPreviews?: UserAttachmentPreview[]
}

interface UseAgentConversationOptions {
  sessionKey?: string | null
  history?: OpenClawChatHistoryMessage[]
  onSessionKeyChange?: (sessionKey: string) => void
}

export function useAgentConversation(
  agentId: string,
  options: UseAgentConversationOptions = {},
) {
  const [turns, setTurns] = useState<AgentConversationTurn[]>([])
  const [streaming, setStreaming] = useState(false)
  const sessionKeyRef = useRef(options.sessionKey ?? '')
  const historyRef = useRef<OpenClawChatHistoryMessage[]>(options.history ?? [])
  const textAccRef = useRef('')
  const thinkAccRef = useRef('')
  const streamAbortRef = useRef<AbortController | null>(null)
  const onSessionKeyChangeRef = useRef(options.onSessionKeyChange)

  useEffect(() => {
    sessionKeyRef.current = options.sessionKey ?? ''
  }, [options.sessionKey])

  useEffect(() => {
    historyRef.current = options.history ?? []
  }, [options.history])

  useEffect(() => {
    onSessionKeyChangeRef.current = options.onSessionKeyChange
  }, [options.onSessionKeyChange])

  useEffect(() => {
    return () => {
      streamAbortRef.current?.abort()
    }
  }, [])

  const updateCurrentTurnParts = (
    updater: (parts: AssistantPart[]) => AssistantPart[],
  ) => {
    setTurns((prev) => {
      const last = prev[prev.length - 1]
      if (!last) return prev
      return [...prev.slice(0, -1), { ...last, parts: updater(last.parts) }]
    })
  }

  const processStreamEvent = (event: OpenClawStreamEvent) => {
    switch (event.type) {
      case 'text-delta': {
        const delta = (event.data.text as string) ?? ''
        textAccRef.current += delta
        const text = textAccRef.current
        updateCurrentTurnParts((parts) => {
          const last = parts[parts.length - 1]
          if (last?.kind === 'text') {
            return [...parts.slice(0, -1), { ...last, text }]
          }
          return [...parts, { kind: 'text', text }]
        })
        break
      }

      case 'thinking': {
        const delta = (event.data.text as string) ?? ''
        thinkAccRef.current += delta
        const text = thinkAccRef.current
        updateCurrentTurnParts((parts) => {
          const idx = parts.findIndex((p) => p.kind === 'thinking' && !p.done)
          if (idx >= 0) {
            return [
              ...parts.slice(0, idx),
              { ...parts[idx], text, done: false },
              ...parts.slice(idx + 1),
            ]
          }
          return [...parts, { kind: 'thinking', text, done: false }]
        })
        break
      }

      case 'tool-start': {
        const rawName = (event.data.toolName as string) ?? 'unknown'
        const args = event.data.args as Record<string, unknown> | undefined
        const { label, subject } = buildToolLabel(rawName, args)
        const tool = {
          id: (event.data.toolCallId as string) ?? crypto.randomUUID(),
          name: rawName,
          label,
          subject,
          status: 'running' as const,
        }
        updateCurrentTurnParts((parts) => {
          const last = parts[parts.length - 1]
          if (last?.kind === 'tool-batch') {
            return [
              ...parts.slice(0, -1),
              { ...last, tools: [...last.tools, tool] },
            ]
          }
          return [...parts, { kind: 'tool-batch', tools: [tool] }]
        })
        break
      }

      case 'tool-end': {
        const toolId = event.data.toolCallId as string
        const toolStatus: 'completed' | 'error' =
          (event.data.status as string) === 'error' ? 'error' : 'completed'
        const durationMs = event.data.durationMs as number | undefined
        updateCurrentTurnParts((parts) => {
          for (let i = parts.length - 1; i >= 0; i--) {
            const part = parts[i]
            if (
              part.kind === 'tool-batch' &&
              part.tools.some((t) => t.id === toolId)
            ) {
              const updatedTools = part.tools.map((t) =>
                t.id === toolId ? { ...t, status: toolStatus, durationMs } : t,
              )
              return [
                ...parts.slice(0, i),
                { ...part, tools: updatedTools },
                ...parts.slice(i + 1),
              ]
            }
          }
          return parts
        })
        break
      }

      case 'done': {
        updateCurrentTurnParts((parts) =>
          parts.map((part) =>
            part.kind === 'thinking' ? { ...part, done: true } : part,
          ),
        )
        setTurns((prev) => {
          const last = prev[prev.length - 1]
          if (!last) return prev
          return [...prev.slice(0, -1), { ...last, done: true }]
        })
        break
      }

      case 'error': {
        const msg =
          (event.data.message as string) ??
          (event.data.error as string) ??
          'Unknown error'
        updateCurrentTurnParts((parts) => [
          ...parts,
          { kind: 'text', text: `Error: ${msg}` },
        ])
        break
      }
    }
  }

  const send = async (input: string | SendInput) => {
    const normalized: SendInput =
      typeof input === 'string' ? { text: input } : input
    const trimmed = normalized.text.trim()
    const attachments = normalized.attachments ?? []
    if (streaming) return
    if (!trimmed && attachments.length === 0) return

    const turn: AgentConversationTurn = {
      id: crypto.randomUUID(),
      userText: trimmed,
      userAttachments:
        normalized.attachmentPreviews &&
        normalized.attachmentPreviews.length > 0
          ? normalized.attachmentPreviews
          : undefined,
      parts: [],
      done: false,
      timestamp: Date.now(),
    }
    setTurns((prev) => [...prev, turn])
    setStreaming(true)
    textAccRef.current = ''
    thinkAccRef.current = ''
    const abortController = new AbortController()
    streamAbortRef.current = abortController

    try {
      const response = await chatWithAgent(
        agentId,
        trimmed,
        sessionKeyRef.current || undefined,
        historyRef.current,
        abortController.signal,
        attachments,
      )
      const responseSessionKey = response.headers.get('X-Session-Key')
      if (responseSessionKey) {
        sessionKeyRef.current = responseSessionKey
        onSessionKeyChangeRef.current?.(responseSessionKey)
      }
      if (!response.ok) {
        const err = await response.text()
        updateCurrentTurnParts((parts) => [
          ...parts,
          { kind: 'text', text: `Error: ${err}` },
        ])
        return
      }
      await consumeSSEStream(
        response,
        processStreamEvent,
        abortController.signal,
      )
    } catch (err) {
      if (abortController.signal.aborted) return
      const msg = err instanceof Error ? err.message : String(err)
      updateCurrentTurnParts((parts) => [
        ...parts,
        { kind: 'text', text: `Error: ${msg}` },
      ])
    } finally {
      if (streamAbortRef.current === abortController) {
        streamAbortRef.current = null
      }
      setStreaming(false)
    }
  }

  const resetConversation = () => {
    streamAbortRef.current?.abort()
    streamAbortRef.current = null
    setTurns([])
    setStreaming(false)
  }

  return {
    turns,
    streaming,
    sessionKey: sessionKeyRef.current,
    send,
    resetConversation,
  }
}
