/**
 * AI SDK ModelMessage → NormalizedBlock adapter.
 *
 * Vendored from pi-vcc's core/normalize.ts and adapted:
 * - Input: AI SDK `ModelMessage` instead of pi's `Message`
 * - Extracts text from parts arrays
 * - Maps tool-call/tool-result to NormalizedBlock
 */
import type { ModelMessage, ToolResultPart } from 'ai'
import { toolResultOutputToText } from '../content'
import { sanitize } from './sanitize'
import type { NormalizedBlock } from './types'

/** Extract plain text from user content (string or parts array). */
function extractUserText(
  content: string | Array<Record<string, unknown>>,
): string {
  if (typeof content === 'string') return content
  const texts: string[] = []
  for (const part of content) {
    if (part.type === 'text' && typeof part.text === 'string') {
      texts.push(part.text)
    }
  }
  return texts.join('\n')
}

/** Extract text and tool calls from assistant content. */
function extractAssistantContent(
  content: string | Array<Record<string, unknown>>,
): {
  texts: string[]
  toolCalls: Array<{ name: string; args: Record<string, unknown> }>
} {
  if (typeof content === 'string') {
    return { texts: [content], toolCalls: [] }
  }
  const texts: string[] = []
  const toolCalls: Array<{ name: string; args: Record<string, unknown> }> = []
  for (const part of content) {
    if (part.type === 'text' && typeof part.text === 'string') {
      texts.push(part.text)
    } else if (part.type === 'tool-call' && typeof part.toolName === 'string') {
      const args: Record<string, unknown> =
        typeof part.input === 'object' && part.input !== null
          ? (part.input as Record<string, unknown>)
          : {}
      toolCalls.push({ name: part.toolName, args })
    }
    // ReasoningPart, FilePart, etc. — skip
  }
  return { texts, toolCalls }
}

/** Check if tool result output is an error type. */
function isToolOutputError(output: ToolResultPart['output']): boolean {
  return output.type === 'error-text' || output.type === 'error-json'
}

/** Convert a single ModelMessage to NormalizedBlock(s). */
// biome-ignore lint/complexity/noExcessiveCognitiveComplexity: handles all ModelMessage variants
function normalizeOne(msg: ModelMessage, msgIndex: number): NormalizedBlock[] {
  // System messages are skipped — they carry no conversation content
  if (msg.role === 'system') return []

  if (msg.role === 'user') {
    const raw = extractUserText(msg.content as any)
    const text = sanitize(raw)
    if (text) {
      return [{ kind: 'user', text, sourceIndex: msgIndex }]
    }
    // Check for image parts
    if (Array.isArray(msg.content)) {
      for (const part of msg.content as unknown as Array<
        Record<string, unknown>
      >) {
        if (part.type === 'image') {
          return [
            {
              kind: 'user',
              text: `[image: ${part.mimeType ?? 'unknown'}]`,
              sourceIndex: msgIndex,
            },
          ]
        }
      }
    }
    return [{ kind: 'user', text: '', sourceIndex: msgIndex }]
  }

  if (msg.role === 'assistant') {
    const { texts, toolCalls } = extractAssistantContent(msg.content as any)
    const blocks: NormalizedBlock[] = []
    for (const t of texts) {
      const sanitized = sanitize(t)
      if (sanitized) {
        blocks.push({
          kind: 'assistant',
          text: sanitized,
          sourceIndex: msgIndex,
        })
      }
    }
    for (const tc of toolCalls) {
      blocks.push({
        kind: 'tool_call',
        name: tc.name,
        args: tc.args,
        sourceIndex: msgIndex,
      })
    }
    return blocks
  }

  if (msg.role === 'tool') {
    const blocks: NormalizedBlock[] = []
    if (Array.isArray(msg.content)) {
      for (const part of msg.content) {
        if (part.type === 'tool-result') {
          const text = sanitize(toolResultOutputToText(part.output))
          blocks.push({
            kind: 'tool_result',
            name: part.toolName ?? 'unknown',
            text,
            isError: isToolOutputError(part.output),
            sourceIndex: msgIndex,
          })
        }
        // ToolApprovalResponse — skip
      }
    }
    return blocks
  }

  return []
}

/** Normalize an array of AI SDK ModelMessages into NormalizedBlocks. */
export function normalizeFromAiSdk(
  messages: ModelMessage[],
): NormalizedBlock[] {
  return messages.flatMap((msg, i) => normalizeOne(msg, i))
}
