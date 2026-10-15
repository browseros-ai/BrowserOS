/**
 * One incremental AI SDK v7 SSE reader for interactive, reconnected and scheduled
 * chats. Cancelling consumption detaches the HTTP subscriber only; callers must
 * use POST /chat/{conversationId}/stop to cancel server work explicitly.
 */
import type { UiMessageChunk } from '@browseros/server-api'
import { ServerApiError } from './errors'
import { isUiMessageChunk } from './generated/ui-message-chunk'

export async function* readUiMessageStream(
  response: Response,
  options: { signal?: AbortSignal } = {},
): AsyncGenerator<UiMessageChunk> {
  if (!response.ok) throw await ServerApiError.fromResponse(response)
  if (response.status === 204) return
  for await (const payload of readSseData(response, options)) {
    if (payload === '[DONE]') return
    let chunk: unknown
    try {
      chunk = JSON.parse(payload)
    } catch {
      throw invalidStream(response, 'Invalid JSON in a chat stream event')
    }
    if (!isUiMessageChunk(chunk)) {
      throw invalidStream(
        response,
        'Invalid UI message chunk in a chat stream event',
      )
    }
    yield chunk
  }
  throw invalidStream(response, 'The chat stream ended before [DONE]')
}

/**
 * Shares framing with non-chat streams such as panel assignments, whose payloads
 * have their own validator and end at EOF rather than a chat [DONE] sentinel.
 * Breaking or aborting consumption closes only this HTTP subscriber.
 */
export async function* readSseData(
  response: Response,
  options: { signal?: AbortSignal } = {},
): AsyncGenerator<string> {
  if (!response.ok) throw await ServerApiError.fromResponse(response)
  if (response.status === 204) return
  if (
    !response.headers
      .get('content-type')
      ?.toLowerCase()
      .startsWith('text/event-stream')
  ) {
    throw invalidStream(response, 'Expected a text/event-stream response')
  }
  if (!response.body)
    throw invalidStream(response, 'The chat stream has no body')
  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  const frames = new SseDataDecoder()
  const { signal } = options
  // Cancel a pending read as well as checking between reads. Cleanup releases
  // the reader even on an early consumer break, without stopping server work.
  const abort = () => {
    void reader.cancel(signal?.reason).catch(() => {})
  }
  signal?.addEventListener('abort', abort, { once: true })
  try {
    while (true) {
      signal?.throwIfAborted()
      const part = await reader.read()
      signal?.throwIfAborted()
      if (part.done) return
      yield* frames.push(decoder.decode(part.value, { stream: true }))
    }
  } finally {
    signal?.removeEventListener('abort', abort)
    await reader.cancel().catch(() => {})
    reader.releaseLock()
  }
}

function invalidStream(response: Response, message: string): ServerApiError {
  return new ServerApiError({
    status: response.status,
    code: 'invalid_stream',
    message,
  })
}

/** Carries SSE line and event state across arbitrary decoded network segments. */
class SseDataDecoder {
  private buffer = ''
  private skipLf = false
  private scanFrom = 0
  private data: string[] = [];

  *push(text: string): Generator<string> {
    this.buffer += text
    let cursor = 0
    // SSE permits LF, CRLF and bare CR. Keep the CR/LF state across reads;
    // TextDecoder separately keeps partial multi-byte UTF-8 characters intact.
    for (let index = this.scanFrom; index < this.buffer.length; index++) {
      const char = this.buffer[index]
      if (this.skipLf) {
        this.skipLf = false
        if (char === '\n') {
          cursor = index + 1
          continue
        }
      }
      if (char !== '\n' && char !== '\r') continue
      const line = this.buffer.slice(cursor, index)
      cursor = index + 1
      this.skipLf = char === '\r'
      if (line === '') {
        if (this.data.length === 0) continue
        const payload = this.data.join('\n')
        this.data = []
        yield payload
      } else if (line === 'data' || line.startsWith('data:')) {
        const value = line === 'data' ? '' : line.slice(5)
        this.data.push(value.startsWith(' ') ? value.slice(1) : value)
      }
    }
    this.buffer = this.buffer.slice(cursor)
    // A large tool-output event may span many reads without a newline. Its
    // retained partial line has already been scanned; only examine new text.
    this.scanFrom = this.buffer.length
  }
}
