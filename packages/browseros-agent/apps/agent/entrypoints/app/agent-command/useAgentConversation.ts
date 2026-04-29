import { useEffect, useRef, useState } from 'react'
import {
  type AgentHarnessStreamEvent,
  attachToHarnessTurn,
  cancelHarnessTurn,
  chatWithHarnessAgent,
  fetchActiveHarnessTurn,
} from '@/entrypoints/app/agents/useAgents'
import type { OpenClawChatHistoryMessage } from '@/entrypoints/app/agents/useOpenClaw'
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
  // The hook always speaks to the harness chat path now; the OpenClaw
  // legacy /claw/agents/:id/chat surface was removed in Step 12. The
  // option remains for forward-compatibility.
  runtime?: 'agent-harness'
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
  // Per-turn resume bookkeeping. `turnId` is captured from the response
  // header; `lastSeq` advances with every SSE event so a reconnect can
  // resume via Last-Event-ID.
  const turnIdRef = useRef<string | null>(null)
  const lastSeqRef = useRef<number | null>(null)

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

  // Indirection for the resume effect below: lets it call the latest
  // event handler without re-subscribing on every render.
  const processEventRef = useRef<(event: AgentHarnessStreamEvent) => void>(
    () => {},
  )

  const updateCurrentTurnParts = (
    updater: (parts: AssistantPart[]) => AssistantPart[],
  ) => {
    setTurns((prev) => {
      const last = prev[prev.length - 1]
      if (!last) return prev
      return [...prev.slice(0, -1), { ...last, parts: updater(last.parts) }]
    })
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
  processEventRef.current = processAgentHarnessStreamEvent

  // On mount (and whenever the agent changes), check whether the
  // server has an in-flight turn for this agent and reattach to it.
  // This is what makes the chat resilient across tab close/reopen,
  // refresh, and navigation: the runtime call kept running on the
  // server while we were away. Effect only depends on `agentId` —
  // the event handler is read off a ref so this doesn't re-subscribe
  // every render.
  useEffect(() => {
    let cancelled = false
    const abortController = new AbortController()

    const attemptResume = async () => {
      try {
        const active = await fetchActiveHarnessTurn(agentId)
        if (cancelled || !active || active.status !== 'running') return
        if (streamAbortRef.current) return // a fresh send already in flight

        // Stage a placeholder turn so the streamed events have a row
        // to render into. We don't have the user message text on
        // resume; the assistant turn is what we're catching up on.
        setTurns((prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            userText: '',
            parts: [],
            done: false,
            timestamp: active.startedAt,
          },
        ])
        textAccRef.current = ''
        thinkAccRef.current = ''
        turnIdRef.current = active.turnId
        lastSeqRef.current = null
        streamAbortRef.current = abortController
        setStreaming(true)

        const response = await attachToHarnessTurn(agentId, {
          turnId: active.turnId,
          signal: abortController.signal,
        })
        if (!response.ok) return
        await consumeSSEStream<AgentHarnessStreamEvent>(
          response,
          (event, meta) => {
            if (typeof meta.seq === 'number') lastSeqRef.current = meta.seq
            processEventRef.current(event)
          },
          abortController.signal,
        )
      } catch {
        // Resume is best-effort; transient errors fall back to the
        // user starting a new turn manually.
      } finally {
        if (!cancelled) {
          if (streamAbortRef.current === abortController) {
            streamAbortRef.current = null
          }
          turnIdRef.current = null
          lastSeqRef.current = null
          setStreaming(false)
        }
      }
    }

    void attemptResume()
    return () => {
      cancelled = true
      abortController.abort()
    }
  }, [agentId])

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
      let response = await chatWithHarnessAgent(
        agentId,
        trimmed,
        abortController.signal,
        attachments,
      )
      // 409 means the server already has an active turn for this
      // agent (e.g. a previous tab kicked one off and we're a fresh
      // mount that missed the resume window). Attach to it instead of
      // double-sending.
      if (response.status === 409) {
        const body = (await response.json()) as { turnId?: string }
        if (body.turnId) {
          response = await attachToHarnessTurn(agentId, {
            turnId: body.turnId,
            signal: abortController.signal,
          })
        }
      }
      const responseSessionKey =
        response.headers.get('X-Session-Key') ??
        response.headers.get('X-Session-Id')
      if (responseSessionKey) {
        sessionKeyRef.current = responseSessionKey
        onSessionKeyChangeRef.current?.(responseSessionKey)
      }
      const responseTurnId = response.headers.get('X-Turn-Id')
      if (responseTurnId) {
        turnIdRef.current = responseTurnId
        lastSeqRef.current = null
      }
      if (!response.ok) {
        const err = await response.text()
        updateCurrentTurnParts((parts) => [
          ...parts,
          { kind: 'text', text: `Error: ${err}` },
        ])
        return
      }
      await consumeSSEStream<AgentHarnessStreamEvent>(
        response,
        (event, meta) => {
          if (typeof meta.seq === 'number') lastSeqRef.current = meta.seq
          processAgentHarnessStreamEvent(event)
        },
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
      turnIdRef.current = null
      lastSeqRef.current = null
      onCompleteRef.current?.()
      setStreaming(false)
    }
  }

  /**
   * Stop button. The fetch abort only detaches *this* SSE subscriber
   * now — the underlying turn would otherwise keep running on the
   * server. So we explicitly cancel via the new endpoint, then unwind
   * the local stream.
   */
  const stop = async () => {
    const turnId = turnIdRef.current ?? undefined
    streamAbortRef.current?.abort()
    streamAbortRef.current = null
    try {
      await cancelHarnessTurn(agentId, {
        turnId,
        reason: 'user pressed stop',
      })
    } catch {
      // Best-effort — UI already aborted.
    }
  }

  const resetConversation = () => {
    void stop()
    setTurns([])
    setStreaming(false)
  }

  return {
    turns,
    streaming,
    sessionKey: sessionKeyRef.current,
    send,
    stop,
    resetConversation,
  }
}
