import { useEffect, useRef, useState } from 'react'
import type {
  AgentConversationTurn,
  AssistantPart,
  ToolEntry,
  UserAttachmentPreview,
} from '@/lib/agent-conversations/types'
import type { ServerAttachmentPayload } from '@/lib/attachments'
import { consumeSSEStream } from '@/lib/sse'
import { buildToolLabel } from '@/lib/tool-labels'
import {
  type AgentHarnessStreamEvent,
  attachToHarnessTurn,
  cancelHarnessTurn,
  chatWithHarnessAgent,
  fetchActiveHarnessTurn,
} from '@/modules/agents/agents.hooks'
import type { AgentChatHistoryMessage } from './agent-chat-types'
import { mapAgentHarnessToolStatus } from './agent-stream-events'

interface SendInput {
  text: string
  attachments?: ServerAttachmentPayload[]
  // Optional preview metadata used to render the optimistic user turn.
  // Built by the composer at staging time; the server only sees the
  // payload array.
  attachmentPreviews?: UserAttachmentPreview[]
}

export interface UseAgentConversationOptions {
  runtime?: 'agent-harness'
  sessionId?: string
  sessionKey?: string | null
  history?: AgentChatHistoryMessage[]
  onComplete?: () => void
  onSessionKeyChange?: (sessionKey: string) => void
  /**
   * Server-side active turn id, surfaced via the listing query. When
   * this changes from null/<id> to a different non-null id while we
   * aren't already streaming (e.g. the server just popped a queued
   * message and started a new turn), the hook reattaches via
   * /chat/active so the chat panel picks up the live stream without
   * waiting for a remount.
   */
  activeTurnId?: string | null
}

export function useAgentConversation(
  agentId: string,
  options: UseAgentConversationOptions = {},
) {
  const [turns, setTurns] = useState<AgentConversationTurn[]>([])
  const [streaming, setStreaming] = useState(false)
  const sessionKeyRef = useRef(options.sessionKey ?? '')
  const historyRef = useRef<AgentChatHistoryMessage[]>(options.history ?? [])
  const textAccRef = useRef('')
  const thinkAccRef = useRef('')
  const streamAbortRef = useRef<AbortController | null>(null)
  const onCompleteRef = useRef(options.onComplete)
  const onSessionKeyChangeRef = useRef(options.onSessionKeyChange)
  const sessionIdRef = useRef(options.sessionId ?? 'main')
  // Per-turn resume bookkeeping. `turnId` is captured from the response
  // header; `lastSeq` advances with every SSE event so a reconnect can
  // resume via Last-Event-ID.
  const turnIdRef = useRef<string | null>(null)
  const lastSeqRef = useRef<number | null>(null)

  useEffect(() => {
    sessionKeyRef.current = options.sessionKey ?? ''
  }, [options.sessionKey])

  useEffect(() => {
    sessionIdRef.current = options.sessionId ?? 'main'
  }, [options.sessionId])

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

  const activeTurnIdDep = options.activeTurnId ?? null

  // On mount, on agent change, and whenever the listing reports a
  // *new* active turn id, check whether the server has an in-flight
  // turn for this agent and reattach to it. This catches three
  // cases at once: the chat resilience flow (tab close/reopen),
  // navigation between agents, AND queue drain (the server starts a
  // new turn from a queued message → activeTurnId flips → attach).
  useEffect(() => {
    let cancelled = false
    const abortController = new AbortController()
    // Reference the dep inside the body so biome's exhaustive-deps
    // rule sees it consumed; the value is just an "any non-null
    // active turn id" trigger — the actual id we attach to comes
    // from the fresh fetchActiveHarnessTurn call below.
    void activeTurnIdDep

    const attemptResume = async () => {
      // Track whether *we* started a stream in this run. When the
      // early-return paths fire (no active turn, or a `send()` /
      // earlier resume already owns `streamAbortRef`), the finally
      // block must NOT touch streaming/turnIdRef/lastSeqRef —
      // otherwise we clobber the in-flight stream's state and the
      // Stop button drops out mid-turn while events keep arriving.
      let weStartedStream = false
      try {
        const active = await fetchActiveHarnessTurn(
          agentId,
          sessionIdRef.current,
        )
        if (cancelled || !active || active.status !== 'running') return
        if (streamAbortRef.current) return // someone else already owns the stream

        // Stage a placeholder turn so the streamed events have a row
        // to render into. The server now persists the kicking-off
        // prompt on the active turn, so we render it as the user
        // bubble immediately — no empty-bubble flicker when a queued
        // message starts running.
        setTurns((prev) => [
          ...prev,
          {
            id: crypto.randomUUID(),
            turnId: active.turnId,
            userText: active.prompt ?? '',
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
        weStartedStream = true

        const response = await attachToHarnessTurn(agentId, {
          sessionId: sessionIdRef.current,
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
        // Always release `streamAbortRef` if we owned it — even when
        // the effect was cancelled mid-stream (a listing poll
        // captured the next queue-drain turn id, for example). If we
        // don't, the next effect run hits `if (streamAbortRef.current)
        // return` against our now-aborted controller and never
        // reattaches, leaving `streaming === true` with no live stream.
        if (weStartedStream && streamAbortRef.current === abortController) {
          streamAbortRef.current = null
        }
        // The other state (streaming flag, turn id, lastSeq) is the
        // *current run's* lifecycle: only reset it on a clean exit.
        // When `cancelled` is true the next run will set these
        // itself, so resetting here would only cause a brief flicker.
        if (!cancelled && weStartedStream) {
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
  }, [agentId, activeTurnIdDep])

  /**
   * Send the chat request and follow the 409-active-turn redirect
   * once. Pulled out of `send` to keep its cognitive complexity in
   * check — the retry adds a branch that biome counts heavily.
   */
  const openSendStream = async (
    targetAgentId: string,
    text: string,
    attachments: ServerAttachmentPayload[],
    signal: AbortSignal,
  ): Promise<Response> => {
    const sessionId = sessionIdRef.current
    const initial = await chatWithHarnessAgent(targetAgentId, text, {
      sessionId,
      signal,
      attachments,
    })
    if (initial.status !== 409) return initial
    // 409 means the server already has an active turn for this agent
    // (a previous tab kicked one off and we're a fresh mount that
    // missed the resume window). Attach to it instead of double-sending.
    const body = (await initial.json()) as { turnId?: string }
    if (!body.turnId) return initial
    return attachToHarnessTurn(targetAgentId, {
      sessionId,
      turnId: body.turnId,
      signal,
    })
  }

  /** Pull session-key / turn-id off response headers and propagate to refs + the optimistic turn. */
  const applyResponseHeadersToTurn = (response: Response) => {
    const responseSessionKey =
      response.headers.get('X-Session-Key') ??
      response.headers.get('X-Session-Id')
    if (responseSessionKey) {
      sessionKeyRef.current = responseSessionKey
      onSessionKeyChangeRef.current?.(responseSessionKey)
    }
    const responseTurnId = response.headers.get('X-Turn-Id')
    if (!responseTurnId) return
    turnIdRef.current = responseTurnId
    lastSeqRef.current = null
    setTurns((prev) => {
      const last = prev[prev.length - 1]
      if (!last) return prev
      return [...prev.slice(0, -1), { ...last, turnId: responseTurnId }]
    })
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
      const response = await openSendStream(
        agentId,
        trimmed,
        attachments,
        abortController.signal,
      )
      applyResponseHeadersToTurn(response)
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
        sessionId: sessionIdRef.current,
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
