import { useEffect, useRef, useState } from 'react'
import {
  buildChatHistoryFromTurns,
  chatWithAgent,
  type OpenClawStreamEvent,
} from '@/entrypoints/app/agents/useOpenClaw'
import {
  getLatestConversation,
  saveConversation,
} from '@/lib/agent-conversations/storage'
import type {
  AgentConversation,
  AgentConversationTurn,
  AssistantPart,
} from '@/lib/agent-conversations/types'
import { consumeSSEStream } from '@/lib/sse'

export function useAgentConversation(agentId: string, agentName: string) {
  const [turns, setTurns] = useState<AgentConversationTurn[]>([])
  const [streaming, setStreaming] = useState(false)
  const [loading, setLoading] = useState(true)
  const sessionKeyRef = useRef('')
  const textAccRef = useRef('')
  const thinkAccRef = useRef('')
  const streamAbortRef = useRef<AbortController | null>(null)

  useEffect(() => {
    let active = true
    getLatestConversation(agentId)
      .then((conv) => {
        if (!active) return
        if (conv) {
          setTurns(conv.turns)
          sessionKeyRef.current = conv.sessionKey
        } else {
          sessionKeyRef.current = crypto.randomUUID()
        }
        setLoading(false)
      })
      .catch(() => {
        if (active) {
          sessionKeyRef.current = crypto.randomUUID()
          setLoading(false)
        }
      })
    return () => {
      active = false
    }
  }, [agentId])

  useEffect(() => {
    return () => {
      streamAbortRef.current?.abort()
    }
  }, [])

  const persistTurns = (updatedTurns: AgentConversationTurn[]) => {
    const conv: AgentConversation = {
      agentId,
      agentName,
      sessionKey: sessionKeyRef.current,
      turns: updatedTurns,
      createdAt: updatedTurns[0]?.timestamp ?? Date.now(),
      updatedAt: Date.now(),
    }
    saveConversation(conv).catch(() => {})
  }

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
        const tool = {
          id: (event.data.toolCallId as string) ?? crypto.randomUUID(),
          name: (event.data.toolName as string) ?? 'unknown',
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
          const updated = [...prev.slice(0, -1), { ...last, done: true }]
          persistTurns(updated)
          return updated
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

  const send = async (text: string) => {
    if (!text.trim() || streaming) return
    const history = buildChatHistoryFromTurns(turns)

    const turn: AgentConversationTurn = {
      id: crypto.randomUUID(),
      userText: text.trim(),
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
        text.trim(),
        sessionKeyRef.current,
        history,
        abortController.signal,
      )
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
    sessionKeyRef.current = crypto.randomUUID()
  }

  return {
    turns,
    streaming,
    loading,
    sessionKey: sessionKeyRef.current,
    send,
    resetConversation,
  }
}
