import {
  ArrowLeft,
  Bot,
  CheckCircle2,
  Loader2,
  Send,
  XCircle,
} from 'lucide-react'
import { type FC, useEffect, useRef, useState } from 'react'
import {
  Message,
  MessageContent,
  MessageResponse,
} from '@/components/ai-elements/message'
import {
  Reasoning,
  ReasoningContent,
  ReasoningTrigger,
} from '@/components/ai-elements/reasoning'
import { Button } from '@/components/ui/button'
import { Textarea } from '@/components/ui/textarea'
import { consumeSSEStream } from '@/lib/sse'
import {
  buildChatHistoryFromTurns,
  chatWithAgent,
  type OpenClawStreamEvent,
} from './useOpenClaw'

interface ToolEntry {
  id: string
  name: string
  status: 'running' | 'completed' | 'error'
  durationMs?: number
}

type AssistantPart =
  | { kind: 'thinking'; text: string; done: boolean }
  | { kind: 'tool-batch'; tools: ToolEntry[] }
  | { kind: 'text'; text: string }

interface ChatTurn {
  id: string
  userText: string
  parts: AssistantPart[]
  done: boolean
}

interface AgentChatProps {
  agentId: string
  agentName: string
  onBack: () => void
}

export const AgentChat: FC<AgentChatProps> = ({
  agentId,
  agentName,
  onBack,
}) => {
  const [turns, setTurns] = useState<ChatTurn[]>([])
  const [input, setInput] = useState('')
  const [streaming, setStreaming] = useState(false)
  const scrollRef = useRef<HTMLDivElement>(null)
  const sessionKeyRef = useRef(crypto.randomUUID())
  const streamAbortRef = useRef<AbortController | null>(null)

  const textAccRef = useRef('')
  const thinkAccRef = useRef('')

  const scrollToBottom = () => {
    scrollRef.current?.scrollTo(0, scrollRef.current.scrollHeight)
  }

  // biome-ignore lint/correctness/useExhaustiveDependencies: scroll on every turns change
  useEffect(() => {
    scrollToBottom()
  }, [turns])

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
        const tool: ToolEntry = {
          id: (event.data.toolCallId as string) ?? crypto.randomUUID(),
          name: (event.data.toolName as string) ?? 'unknown',
          status: 'running',
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
        const status =
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
                t.id === toolId
                  ? {
                      ...t,
                      status: status as ToolEntry['status'],
                      durationMs,
                    }
                  : t,
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

  const handleSend = async () => {
    const text = input.trim()
    if (!text || streaming) return
    const history = buildChatHistoryFromTurns(turns)

    const turn: ChatTurn = {
      id: crypto.randomUUID(),
      userText: text,
      parts: [],
      done: false,
    }
    setTurns((prev) => [...prev, turn])
    setInput('')
    setStreaming(true)

    textAccRef.current = ''
    thinkAccRef.current = ''
    const abortController = new AbortController()
    streamAbortRef.current = abortController

    try {
      const response = await chatWithAgent(
        agentId,
        text,
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

  return (
    <div className="flex h-[calc(100vh-4rem)] flex-col">
      <div className="flex items-center gap-2 border-b px-4 py-3">
        <Button variant="ghost" size="icon" onClick={onBack}>
          <ArrowLeft className="size-4" />
        </Button>
        <h2 className="font-semibold text-lg">{agentName}</h2>
      </div>

      <div ref={scrollRef} className="flex-1 space-y-4 overflow-y-auto p-4">
        {turns.map((turn) => (
          <div key={turn.id} className="space-y-3">
            {/* User message */}
            <Message from="user">
              <MessageContent>
                <pre className="whitespace-pre-wrap font-sans text-sm">
                  {turn.userText}
                </pre>
              </MessageContent>
            </Message>

            {/* Assistant response — all parts grouped */}
            {turn.parts.length > 0 && (
              <Message from="assistant">
                <MessageContent>
                  {turn.parts.map((part, i) => {
                    const key = `${turn.id}-part-${i}`

                    switch (part.kind) {
                      case 'thinking':
                        return (
                          <Reasoning
                            key={key}
                            className="w-full"
                            isStreaming={!part.done}
                            defaultOpen={!part.done}
                          >
                            <ReasoningTrigger />
                            <ReasoningContent>{part.text}</ReasoningContent>
                          </Reasoning>
                        )

                      case 'tool-batch':
                        return (
                          <div key={key} className="w-full space-y-1">
                            {part.tools.map((tool) => (
                              <div
                                key={tool.id}
                                className="flex items-center gap-2 rounded-md border px-3 py-2 text-sm"
                              >
                                {tool.status === 'running' && (
                                  <Loader2 className="size-3.5 animate-spin text-muted-foreground" />
                                )}
                                {tool.status === 'completed' && (
                                  <CheckCircle2 className="size-3.5 text-green-500" />
                                )}
                                {tool.status === 'error' && (
                                  <XCircle className="size-3.5 text-destructive" />
                                )}
                                <span className="font-mono text-xs">
                                  {tool.name}
                                </span>
                                {tool.durationMs != null && (
                                  <span className="ml-auto text-muted-foreground text-xs">
                                    {(tool.durationMs / 1000).toFixed(1)}s
                                  </span>
                                )}
                              </div>
                            ))}
                          </div>
                        )

                      case 'text':
                        return (
                          <MessageResponse key={key}>
                            {part.text}
                          </MessageResponse>
                        )
                      default:
                        return null
                    }
                  })}
                </MessageContent>
              </Message>
            )}

            {/* Streaming indicator when waiting for first part */}
            {!turn.done && turn.parts.length === 0 && streaming && (
              <div className="flex gap-2">
                <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-full bg-[var(--accent-orange)] text-white">
                  <Bot className="h-3.5 w-3.5" />
                </div>
                <div className="flex items-center gap-1 rounded-xl rounded-tl-none border border-border/50 bg-card px-3 py-2.5 shadow-sm">
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-[var(--accent-orange)] [animation-delay:-0.3s]" />
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-[var(--accent-orange)] [animation-delay:-0.15s]" />
                  <span className="h-1.5 w-1.5 animate-bounce rounded-full bg-[var(--accent-orange)]" />
                </div>
              </div>
            )}
          </div>
        ))}
      </div>

      <div className="border-t p-4">
        <div className="flex gap-2">
          <Textarea
            value={input}
            onChange={(e) => setInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && !e.shiftKey) {
                e.preventDefault()
                handleSend()
              }
            }}
            placeholder="Send a message..."
            className="min-h-[44px] resize-none"
            rows={1}
          />
          <Button
            onClick={handleSend}
            disabled={!input.trim() || streaming}
            size="icon"
          >
            {streaming ? (
              <Loader2 className="size-4 animate-spin" />
            ) : (
              <Send className="size-4" />
            )}
          </Button>
        </div>
      </div>
    </div>
  )
}
