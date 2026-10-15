/**
 * Keeps the AI SDK's request preparation, HTTP handling and React reducer, while
 * all interactive, resumed and scheduled chats use the contract's SSE reader.
 * Cancelling this stream detaches the view; POST /stop owns run cancellation.
 */
import type { UiMessageChunk } from '@browseros/server-api'
import { readUiMessageStream } from '@browseros/server-api-client'
import { DefaultChatTransport, type UIMessage, type UIMessageChunk } from 'ai'

export class ServerChatTransport extends DefaultChatTransport<UIMessage> {
  protected override processResponseStream(
    body: ReadableStream<Uint8Array<ArrayBufferLike>>,
  ): ReadableStream<UIMessageChunk> {
    // HttpChatTransport has already handled non-2xx/204. This wraps the existing
    // body without fetching or buffering it a second time.
    const detach = new AbortController()
    const chunks = readUiMessageStream(
      new Response(body, {
        headers: { 'content-type': 'text/event-stream' },
      }),
      { signal: detach.signal },
    )
    let cancelled = false
    return new ReadableStream<UIMessageChunk>({
      async pull(controller) {
        try {
          const next = await chunks.next()
          if (cancelled) return
          if (next.done) controller.close()
          else {
            if (!hasSdkMetadata(next.value))
              throw new Error('Invalid UI message metadata')
            controller.enqueue(next.value)
          }
        } catch (error) {
          if (!cancelled) controller.error(error)
        }
      },
      async cancel(reason) {
        cancelled = true
        // Abort also interrupts a currently pending next(), so iterator.return
        // cannot hang waiting for another server event during panel teardown.
        detach.abort(reason)
        await chunks.return(undefined)
      },
    })
  }
}

type JsonValue =
  | null
  | string
  | number
  | boolean
  | JsonValue[]
  | { [key: string]: JsonValue | undefined }
function isJson(value: unknown): value is JsonValue {
  return (
    value === null ||
    typeof value === 'string' ||
    typeof value === 'boolean' ||
    (typeof value === 'number' && Number.isFinite(value)) ||
    (Array.isArray(value)
      ? value.every(isJson)
      : typeof value === 'object' && Object.values(value).every(isJson))
  )
}

/**
 * SPEC-GAP: C1 models nested metadata as unknown, whereas the installed SDK
 * requires JSON values. C1 validates the discriminated union; this refinement
 * checks only that narrower metadata constraint and preserves every field.
 */
function hasSdkMetadata(
  chunk: UiMessageChunk,
): chunk is UiMessageChunk & UIMessageChunk {
  return (
    (!('providerMetadata' in chunk) || isJson(chunk.providerMetadata)) &&
    (!('toolMetadata' in chunk) || isJson(chunk.toolMetadata))
  )
}
