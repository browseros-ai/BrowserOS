import { useEffect, useRef, useState } from 'react'
import {
  type AgentHarnessStreamEvent,
  chatWithHarnessAgent,
} from '@/entrypoints/app/agents/useAgents'
import {
  chatWithAgent,
  type OpenClawChatHistoryMessage,
  type OpenClawStreamEvent,
} from '@/entrypoints/app/agents/useOpenClaw'
import type {
  AgentConversationTurn,
  AssistantPart,
  ToolEntry,
  UserAttachmentPreview,
} from '@/lib/agent-conversations/types'
import type { ServerAttachmentPayload } from '@/lib/attachments'
import { consumeSSEStream } from '@/lib/sse'
import { buildToolLabel } from '@/lib/tool-labels'
import { mapAgentHarnessToolStatus } from './agent-stream-events'

export interface SendInput {
  text: string
  attachments?: ServerAttachmentPayload[]
  // Optional preview metadata used to render the optimistic user turn.
  // Built by the composer at staging time; the server only sees the
  // payload array.
  attachmentPreviews?: UserAttachmentPreview[]
}

interface UseAgentConversationOptions {
  runtime?: 'openclaw' | 'agent-harness'
  sessionKey?: string | null
  history?: OpenClawChatHistoryMessage[]
  onComplete?: () => void
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
  const onCompleteRef = useRef(options.onComplete)
  const onSessionKeyChangeRef = useRef(options.onSessionKeyChange)

  useEffect(() => {
    sessionKeyRef.current = options.sessionKey ?? ''
  }, [options.sessionKey])

  useEffect(() => {
    historyRef.current = options.history ?? []
  }, [options.history])

  useEffect(() => {
    onCompleteRef.current = options.onComplete
  }, [options.onComplete])

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
        appendTextDelta((event.data.text as string) ?? '')
        break
      }

      case 'thinking': {
        appendThinkingDelta((event.data.text as string) ?? '')
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
        markCurrentTurnDone()
        break
      }

      case 'error': {
        const msg =
          (event.data.message as string) ??
          (event.data.error as string) ??
          'Unknown error'
        appendErrorText(msg)
        break
      }
    }
  }

  const appendTextDelta = (delta: string) => {
    textAccRef.current += delta
    const text = textAccRef.current
    updateCurrentTurnParts((parts) => {
      const last = parts[parts.length - 1]
      if (last?.kind === 'text') {
        return [...parts.slice(0, -1), { ...last, text }]
      }
      return [...parts, { kind: 'text', text }]
    })
  }

  const appendThinkingDelta = (delta: string) => {
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
  }

  const appendErrorText = (message: string) => {
    updateCurrentTurnParts((parts) => [
      ...parts,
      { kind: 'text', text: `Error: ${message}` },
    ])
  }

  const markCurrentTurnDone = () => {
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
  }

  const upsertAgentHarnessTool = (event: AgentHarnessStreamEvent) => {
    if (event.type !== 'tool_call') return
    const rawName = event.title || event.rawType || 'tool call'
    const { label, subject } = buildToolLabel(
      rawName,
      event.text ? { description: event.text } : undefined,
    )
    const tool: ToolEntry = {
      id: event.id ?? crypto.randomUUID(),
      name: rawName,
      label,
      subject,
      status: mapAgentHarnessToolStatus(event.status),
    }

    updateCurrentTurnParts((parts) => {
      for (let i = parts.length - 1; i >= 0; i--) {
        const part = parts[i]
        if (
          part.kind === 'tool-batch' &&
          part.tools.some((existing) => existing.id === tool.id)
        ) {
          const tools = part.tools.map((existing) =>
            existing.id === tool.id ? { ...existing, ...tool } : existing,
          )
          return [
            ...parts.slice(0, i),
            { ...part, tools },
            ...parts.slice(i + 1),
          ]
        }
      }

      const last = parts[parts.length - 1]
      if (last?.kind === 'tool-batch') {
        return [
          ...parts.slice(0, -1),
          { ...last, tools: [...last.tools, tool] },
        ]
      }
      return [...parts, { kind: 'tool-batch', tools: [tool] }]
    })
  }

  const processAgentHarnessStreamEvent = (event: AgentHarnessStreamEvent) => {
    switch (event.type) {
      case 'text_delta':
        if (event.stream === 'thought') {
          appendThinkingDelta(event.text)
        } else {
          appendTextDelta(event.text)
        }
        break
      case 'tool_call':
        upsertAgentHarnessTool(event)
        break
      case 'done':
        markCurrentTurnDone()
        break
      case 'error':
        appendErrorText(event.message)
        break
      case 'status':
        break
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
      const response =
        options.runtime === 'agent-harness'
          ? await chatWithHarnessAgent(agentId, trimmed, abortController.signal)
          : await chatWithAgent(
              agentId,
              trimmed,
              sessionKeyRef.current || undefined,
              historyRef.current,
              abortController.signal,
              attachments,
            )
      const responseSessionKey =
        response.headers.get('X-Session-Key') ??
        response.headers.get('X-Session-Id')
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
      if (options.runtime === 'agent-harness') {
        await consumeSSEStream<AgentHarnessStreamEvent>(
          response,
          processAgentHarnessStreamEvent,
          abortController.signal,
        )
      } else {
        await consumeSSEStream<OpenClawStreamEvent>(
          response,
          processStreamEvent,
          abortController.signal,
        )
      }
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
      onCompleteRef.current?.()
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
