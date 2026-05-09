import { useCallback, useEffect, useRef, useState } from 'react'
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
  ConversationTurnFile,
  ToolEntry,
  UserAttachmentPreview,
} from '@/lib/agent-conversations/types'
import { useInvalidateAgentOutputs } from '@/lib/agent-files'
import type { ServerAttachmentPayload } from '@/lib/attachments'
import { sentry } from '@/lib/sentry/sentry'
import { consumeSSEStream } from '@/lib/sse'
import { buildToolLabel } from '@/lib/tool-labels'
import {
  createWorkflowUsageRecord,
  recordWorkflowUsage,
} from '@/lib/workflow-usage/storage'
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
  const invalidateAgentOutputs = useInvalidateAgentOutputs()
  // Stable ref so the resume effect doesn't re-subscribe on every
  // render (the hook's returned callable is freshly closured each
  // time, but the underlying queryClient is stable).
  const invalidateAgentOutputsRef = useRef(invalidateAgentOutputs)
  invalidateAgentOutputsRef.current = invalidateAgentOutputs
  const sessionKeyRef = useRef(options.sessionKey ?? '')
  const historyRef = useRef<OpenClawChatHistoryMessage[]>(options.history ?? [])
  const textAccRef = useRef('')
  const thinkAccRef = useRef('')
  const streamAbortRef = useRef<AbortController | null>(null)
  const onCompleteRef = useRef(options.onComplete)
  const onSessionKeyChangeRef = useRef(options.onSessionKeyChange)
  const workflowToolNamesRef = useRef<string[]>([])
  const workflowToolIdsRef = useRef(new Set<string>())
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

  const resetWorkflowUsageCapture = useCallback(() => {
    workflowToolNamesRef.current = []
    workflowToolIdsRef.current = new Set()
  }, [])

  const persistWorkflowUsageCapture = useCallback(
    (turnId?: string | null) => {
      const toolNames = workflowToolNamesRef.current
      if (toolNames.length === 0) return

      void recordWorkflowUsage(
        createWorkflowUsageRecord({
          id: `agent-harness-turn:${turnId ?? crypto.randomUUID()}`,
          source: 'agent-harness-chat',
          toolNames,
        }),
      ).catch((error) => {
        sentry.captureException(error, {
          extra: {
            message: 'Failed to persist agent workflow usage pattern',
            agentId,
            turnId,
          },
        })
      })
    },
    [agentId],
  )

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

  const setProducedFilesOnCurrentTurn = (files: ConversationTurnFile[]) => {
    setTurns((prev) => {
      const last = prev[prev.length - 1]
      if (!last) return prev
      // Replace, don't merge: the server's diff is authoritative for
      // the just-completed turn — duplicate events shouldn't grow the
      // list, and a re-attribution should overwrite an earlier one.
      return [...prev.slice(0, -1), { ...last, producedFiles: files }]
    })
  }

  const upsertAgentHarnessTool = (event: AgentHarnessStreamEvent) => {
    if (event.type !== 'tool_call') return
    const rawName = event.title || event.rawType || 'tool call'
    const toolId = event.id ?? rawName
    if (!workflowToolIdsRef.current.has(toolId)) {
      workflowToolIdsRef.current.add(toolId)
      workflowToolNamesRef.current.push(rawName)
    }
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
      case 'produced_files':
        setProducedFilesOnCurrentTurn(event.files)
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
        const active = await fetchActiveHarnessTurn(agentId)
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
        resetWorkflowUsageCapture()

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
          const finishedTurnId = turnIdRef.current
          persistWorkflowUsageCapture(finishedTurnId)
          turnIdRef.current = null
          lastSeqRef.current = null
          setStreaming(false)
          void invalidateAgentOutputsRef.current(
            agentId,
            finishedTurnId ?? undefined,
          )
        }
      }
    }

    void attemptResume()
    return () => {
      cancelled = true
      abortController.abort()
    }
  }, [
    agentId,
    activeTurnIdDep,
    persistWorkflowUsageCapture,
    resetWorkflowUsageCapture,
  ])

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
    const initial = await chatWithHarnessAgent(
      targetAgentId,
      text,
      signal,
      attachments,
    )
    if (initial.status !== 409) return initial
    // 409 means the server already has an active turn for this agent
    // (a previous tab kicked one off and we're a fresh mount that
    // missed the resume window). Attach to it instead of double-sending.
    const body = (await initial.json()) as { turnId?: string }
    if (!body.turnId) return initial
    return attachToHarnessTurn(targetAgentId, {
      turnId: body.turnId,
      signal,
    })
  }

  /**
   * Pull session-key / turn-id off response headers and propagate to
   * refs + the optimistic turn. Stamping `turnId` here lets the
   * inline artifact card fall back to /files/turn/<id> on a resumed
   * mount that missed the live `produced_files` event.
   */
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
    resetWorkflowUsageCapture()
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
      // Capture before nulling — the invalidation needs the turn id so
      // useAgentTurnFiles consumers also flush, not just the agent-wide
      // rail query.
      const finishedTurnId = turnIdRef.current
      persistWorkflowUsageCapture(finishedTurnId)
      turnIdRef.current = null
      lastSeqRef.current = null
      onCompleteRef.current?.()
      setStreaming(false)
      void invalidateAgentOutputs(agentId, finishedTurnId ?? undefined)
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
