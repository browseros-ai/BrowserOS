function isAbortError(error: unknown): boolean {
  return error instanceof DOMException && error.name === 'AbortError'
}

interface ParsedSSEEvent<T> {
  data: T
  /** Numeric `id:` line on the same SSE event, if any. */
  seq?: number
}

function parseSSELines<T>(buffer: string): {
  events: ParsedSSEEvent<T>[]
  remainder: string
} {
  // SSE events are separated by blank lines. Buffer lines until we hit
  // a blank, then assemble each event. Lines we recognise: `id: <n>`
  // and `data: <payload>`. Everything else is ignored.
  const events: ParsedSSEEvent<T>[] = []
  const lines = buffer.split('\n')
  // Find the last blank-line boundary; everything after it is the
  // remainder (next event partially received).
  let lastBoundary = -1
  for (let i = lines.length - 1; i >= 0; i--) {
    if (lines[i] === '') {
      lastBoundary = i
      break
    }
  }
  const completeLines = lastBoundary >= 0 ? lines.slice(0, lastBoundary) : []
  const remainder =
    lastBoundary >= 0 ? lines.slice(lastBoundary + 1).join('\n') : buffer

  let currentSeq: number | undefined
  let currentData: string | null = null
  const flush = () => {
    if (currentData != null && currentData !== '[DONE]') {
      try {
        events.push({
          data: JSON.parse(currentData) as T,
          seq: currentSeq,
        })
      } catch {
        // ignore
      }
    }
    currentSeq = undefined
    currentData = null
  }

  for (const line of completeLines) {
    if (line === '') {
      flush()
      continue
    }
    if (line.startsWith('id: ')) {
      const n = Number.parseInt(line.slice(4).trim(), 10)
      if (Number.isFinite(n)) currentSeq = n
      continue
    }
    if (line.startsWith('data: ')) {
      currentData = line.slice(6)
    }
  }
  // Catch a complete trailing event with no terminating blank line —
  // shouldn't happen in well-formed SSE, but be tolerant.
  flush()

  return { events, remainder }
}

export async function consumeSSEStream<T>(
  response: Response,
  onEvent: (event: T, meta: { seq?: number }) => void,
  signal?: AbortSignal,
): Promise<void> {
  const reader = response.body?.getReader()
  if (!reader) return

  const decoder = new TextDecoder()
  let buffer = ''

  const abortReader = () => {
    void reader.cancel()
  }

  signal?.addEventListener('abort', abortReader, { once: true })

  try {
    while (true) {
      const { done, value } = await reader.read()
      if (done) break

      buffer += decoder.decode(value, { stream: true })
      const { events, remainder } = parseSSELines<T>(buffer)
      buffer = remainder

      for (const event of events) {
        onEvent(event.data, { seq: event.seq })
      }
    }
  } catch (error) {
    if (signal?.aborted || isAbortError(error)) return
    throw error
  } finally {
    signal?.removeEventListener('abort', abortReader)
    const trailing = decoder.decode()
    if (trailing) {
      buffer += trailing
    }
    if (buffer) {
      const { events } = parseSSELines<T>(buffer)
      for (const event of events) {
        onEvent(event.data, { seq: event.seq })
      }
    }
  }
}
