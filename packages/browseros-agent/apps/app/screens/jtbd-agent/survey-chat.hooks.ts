import { useRef, useState } from 'react'
import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { BROWSEROS_PREFS } from '@/lib/browseros/prefs'

const JTBD_API_URL = 'https://jtbd-agent.fly.dev'
// const LOCAL_JTBD_API_URL = 'http://localhost:3001'
const DEFAULT_MAX_TURNS = 20
const DEFAULT_EXPERIMENT_ID = 'default'

export interface SurveyChatOptions {
  maxTurns?: number
  experimentId?: string
}

async function getInstallId(): Promise<string> {
  try {
    const adapter = getBrowserOSAdapter()
    const pref = await adapter.getPref(BROWSEROS_PREFS.INSTALL_ID)
    if (pref?.value) {
      return String(pref.value)
    }
  } catch {
    // BrowserOS API not available
  }
  return ''
}

export type Message = {
  id: string
  role: 'user' | 'assistant'
  content: string
}

type Phase = 'idle' | 'active' | 'completed' | 'error'

const INTERVIEW_COMPLETE_MARKER = '__INTERVIEW_COMPLETE__'

// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: JTBD agent is temporary
async function* streamSSE(
  response: Response,
): AsyncGenerator<string, void, unknown> {
  const reader = response.body?.getReader()
  if (!reader) throw new Error('Response body is not readable')

  const decoder = new TextDecoder()
  let buffer = ''

  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break

      buffer += decoder.decode(value, { stream: true })
      const lines = buffer.split('\n')
      buffer = lines.pop() ?? ''

      for (const line of lines) {
        if (!line.startsWith('data: ')) continue
        const data = line.slice(6)
        if (data === '[DONE]') return

        try {
          const event = JSON.parse(data)
          if (event.type === 'text-delta' && event.delta) {
            yield event.delta
          } else if (event.type === 'interview_complete') {
            yield INTERVIEW_COMPLETE_MARKER
          } else if (event.type === 'error' && event.errorText) {
            throw new Error(event.errorText)
          }
        } catch (e) {
          if (
            e instanceof Error &&
            e.message !== 'Unexpected end of JSON input'
          ) {
            throw e
          }
        }
      }
    }
  } finally {
    reader.releaseLock()
  }
}

export function useChat(options: SurveyChatOptions = {}) {
  const maxTurns = options.maxTurns ?? DEFAULT_MAX_TURNS
  const experimentId = options.experimentId ?? DEFAULT_EXPERIMENT_ID

  const [phase, setPhase] = useState<Phase>('idle')
  const [messages, setMessages] = useState<Message[]>([])
  const [isStreaming, setIsStreaming] = useState(false)
  const [error, setError] = useState<Error | null>(null)
  const sessionIdRef = useRef<string | null>(null)
  const abortControllerRef = useRef<AbortController | null>(null)

  const appendMessage = (msg: Message) => {
    setMessages((prev) => [...prev, msg])
  }

  const updateLastMessage = (content: string) => {
    setMessages((prev) => {
      if (prev.length === 0) return prev
      const updated = [...prev]
      updated[updated.length - 1] = {
        ...updated[updated.length - 1],
        content,
      }
      return updated
    })
  }

  const start = async () => {
    setPhase('active')
    setError(null)
    setIsStreaming(true)

    const assistantMsgId = crypto.randomUUID()
    appendMessage({ id: assistantMsgId, role: 'assistant', content: '' })

    abortControllerRef.current = new AbortController()

    try {
      const installId = await getInstallId()
      const response = await fetch(`${JTBD_API_URL}/api/interview/start`, {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ installId, experimentId, maxTurns }),
        signal: abortControllerRef.current.signal,
      })

      if (!response.ok) {
        throw new Error(`Failed to start interview: ${response.status}`)
      }

      const newSessionId = response.headers.get('x-interview-session-id')
      if (newSessionId) {
        sessionIdRef.current = newSessionId
      } else {
        const err = new Error('No session ID returned from server')
        setError(err)
        setPhase('error')
        return
      }

      let accumulated = ''
      for await (const chunk of streamSSE(response)) {
        accumulated += chunk
        updateLastMessage(accumulated)
      }
    } catch (e) {
      if (e instanceof Error && e.name === 'AbortError') return
      const err = e instanceof Error ? e : new Error('Unknown error')
      setError(err)
      setPhase('error')
    } finally {
      setIsStreaming(false)
      abortControllerRef.current = null
    }
  }

  const respond = async (text: string) => {
    if (!sessionIdRef.current) return

    const userMsgId = crypto.randomUUID()
    appendMessage({ id: userMsgId, role: 'user', content: text })

    const assistantMsgId = crypto.randomUUID()
    appendMessage({ id: assistantMsgId, role: 'assistant', content: '' })

    setIsStreaming(true)
    setError(null)

    abortControllerRef.current = new AbortController()

    try {
      const response = await fetch(
        `${JTBD_API_URL}/api/interview/${sessionIdRef.current}/respond`,
        {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ response: text, maxTurns }),
          signal: abortControllerRef.current.signal,
        },
      )

      if (!response.ok) {
        throw new Error(`Failed to respond: ${response.status}`)
      }

      let accumulated = ''
      let isComplete = false

      for await (const chunk of streamSSE(response)) {
        if (chunk === INTERVIEW_COMPLETE_MARKER) {
          isComplete = true
          continue
        }
        accumulated += chunk
        updateLastMessage(accumulated)
      }

      if (isComplete) {
        setPhase('completed')
      }
    } catch (e) {
      if (e instanceof Error && e.name === 'AbortError') return
      const err = e instanceof Error ? e : new Error('Unknown error')
      setError(err)
      setPhase('error')
    } finally {
      setIsStreaming(false)
      abortControllerRef.current = null
    }
  }

  const stop = () => {
    abortControllerRef.current?.abort()
    setIsStreaming(false)
  }

  const reset = () => {
    stop()
    setMessages([])
    setError(null)
    sessionIdRef.current = null
    setPhase('idle')
  }

  return {
    phase,
    messages,
    isStreaming,
    error,
    start,
    respond,
    stop,
    reset,
  }
}
