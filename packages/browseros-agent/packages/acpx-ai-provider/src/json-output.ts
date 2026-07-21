import type { LanguageModelV2StreamPart } from '@ai-sdk/provider'

const OPENING_FENCE_PATTERN = /^```(?:\w+)?\s*\n/
const CLOSING_FENCE_PATTERN = /\n?```\s*$/
const SUFFIX_BUFFER_SIZE = 12

type TextStartPart = Extract<LanguageModelV2StreamPart, { type: 'text-start' }>

interface TextBlockState {
  startEvent: TextStartPart
  phase: 'prefix' | 'streaming'
  buffer: string
  prefixStripped: boolean
}

/**
 * Strips ```json / ``` markdown fences if the text is wrapped in them.
 * Returns the inner content trimmed; if no full fence pair is present,
 * returns the original input trimmed unchanged.
 */
export function stripMarkdownFences(text: string): string {
  const trimmed = text.trim()
  const match = trimmed.match(/^```(?:\w+)?\s*\n?([\s\S]*?)\n?\s*```\s*$/)
  if (match?.[1] !== undefined) {
    return match[1].trim()
  }
  return trimmed
}

function emitTextDelta(
  controller: TransformStreamDefaultController<LanguageModelV2StreamPart>,
  id: string,
  delta: string,
): void {
  if (!delta) return
  controller.enqueue({ type: 'text-delta', id, delta })
}

function finalizeBufferedText(block: TextBlockState): string {
  if (block.prefixStripped) {
    return block.buffer.replace(CLOSING_FENCE_PATTERN, '').trimEnd()
  }
  return stripMarkdownFences(block.buffer)
}

function advancePrefix(
  block: TextBlockState,
  controller: TransformStreamDefaultController<LanguageModelV2StreamPart>,
): void {
  if (block.buffer.length === 0) return

  if (!block.buffer.startsWith('`')) {
    block.phase = 'streaming'
    controller.enqueue(block.startEvent)
    return
  }

  if (block.buffer.startsWith('```')) {
    if (!block.buffer.includes('\n')) return
    const prefixMatch = block.buffer.match(OPENING_FENCE_PATTERN)
    block.phase = 'streaming'
    if (prefixMatch) {
      block.buffer = block.buffer.slice(prefixMatch[0].length)
      block.prefixStripped = true
    }
    controller.enqueue(block.startEvent)
    return
  }

  if (block.buffer.length >= 3) {
    block.phase = 'streaming'
    controller.enqueue(block.startEvent)
  }
}

type Controller = TransformStreamDefaultController<LanguageModelV2StreamPart>

function handleTextDelta(
  blocks: Record<string, TextBlockState>,
  chunk: Extract<LanguageModelV2StreamPart, { type: 'text-delta' }>,
  controller: Controller,
): void {
  const block = blocks[chunk.id]
  if (!block) {
    controller.enqueue(chunk)
    return
  }

  block.buffer += chunk.delta

  if (block.phase === 'prefix') {
    advancePrefix(block, controller)
  }

  if (block.phase === 'streaming' && block.buffer.length > SUFFIX_BUFFER_SIZE) {
    const toStream = block.buffer.slice(0, -SUFFIX_BUFFER_SIZE)
    block.buffer = block.buffer.slice(-SUFFIX_BUFFER_SIZE)
    emitTextDelta(controller, chunk.id, toStream)
  }
}

function handleTextEnd(
  blocks: Record<string, TextBlockState>,
  chunk: Extract<LanguageModelV2StreamPart, { type: 'text-end' }>,
  controller: Controller,
): void {
  const block = blocks[chunk.id]
  if (!block) {
    controller.enqueue(chunk)
    return
  }

  if (block.phase === 'prefix') {
    controller.enqueue(block.startEvent)
  }
  emitTextDelta(controller, chunk.id, finalizeBufferedText(block))
  controller.enqueue(chunk)
  delete blocks[chunk.id]
}

/**
 * TransformStream that strips ```json / ``` markdown fences from streamed
 * text-deltas while preserving incremental streaming.
 *
 * Strategy:
 * 1. Hold the `text-start` briefly while we decide whether the first bytes
 *    are a fence or raw JSON.
 * 2. If an opening fence is present, strip it once we have enough bytes to
 *    recognize it (need a newline to know where the language tag ends).
 * 3. While streaming, keep a tiny suffix buffer so a trailing closing fence
 *    can be removed without buffering the full payload.
 * 4. Pass through every non-text part untouched.
 */
export function createJsonCleanupTransform(): TransformStream<
  LanguageModelV2StreamPart,
  LanguageModelV2StreamPart
> {
  const blocks: Record<string, TextBlockState> = {}

  return new TransformStream<
    LanguageModelV2StreamPart,
    LanguageModelV2StreamPart
  >({
    transform(chunk, controller) {
      switch (chunk.type) {
        case 'text-start':
          blocks[chunk.id] = {
            startEvent: chunk,
            phase: 'prefix',
            buffer: '',
            prefixStripped: false,
          }
          return
        case 'text-delta':
          handleTextDelta(blocks, chunk, controller)
          return
        case 'text-end':
          handleTextEnd(blocks, chunk, controller)
          return
        default:
          controller.enqueue(chunk)
      }
    },

    flush(controller) {
      for (const [id, block] of Object.entries(blocks)) {
        if (block.phase === 'prefix') {
          controller.enqueue(block.startEvent)
        }
        emitTextDelta(controller, id, finalizeBufferedText(block))
      }
    },
  })
}
