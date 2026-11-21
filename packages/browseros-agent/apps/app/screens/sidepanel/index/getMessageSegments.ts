import type { UIMessage } from 'ai'

export type ToolInvocationState =
  | 'partial-call'
  | 'call'
  | 'result'
  | 'input-streaming'
  | 'input-available'
  | 'output-available'
  | 'output-error'
  | 'approval-requested'
  | 'approval-responded'
  | 'output-denied'

export interface ToolInvocationInfo {
  state: ToolInvocationState
  toolCallId: string
  toolName: string
  input: Record<string, unknown>
  output: unknown
  approval?: { id: string; approved?: boolean; reason?: string }
}

export type MessageSegment =
  | { type: 'text'; key: string; text: string }
  | { type: 'reasoning'; key: string; text: string; isStreaming: boolean }
  | { type: 'tool-batch'; key: string; tools: ToolInvocationInfo[] }

// Nudge tools the old server still emits and stored conversations still hold.
// Their cards are gone, so they are skipped rather than shown as tool rows.
const HIDDEN_TOOLS = new Set(['suggest_schedule', 'suggest_app_connection'])

export const getMessageSegments = (
  message: UIMessage,
  isLastMessage: boolean,
  isStreaming: boolean,
): MessageSegment[] => {
  const segments: MessageSegment[] = []
  let currentToolBatch: ToolInvocationInfo[] = []
  let textSegmentCount = 0
  let reasoningSegmentCount = 0

  const flushToolBatch = () => {
    if (currentToolBatch.length > 0) {
      segments.push({
        type: 'tool-batch',
        key: `${message.id}-tools-${currentToolBatch[0].toolCallId}`,
        tools: [...currentToolBatch],
      })
      currentToolBatch = []
    }
  }

  for (let i = 0; i < message.parts.length; i++) {
    const part = message.parts[i]

    if (part.type === 'text') {
      flushToolBatch()
      segments.push({
        type: 'text',
        key: `${message.id}-text-${textSegmentCount}`,
        text: part.text,
      })
      textSegmentCount++
    } else if (part.type === 'reasoning') {
      flushToolBatch()
      segments.push({
        type: 'reasoning',
        key: `${message.id}-reasoning-${reasoningSegmentCount}`,
        text: part.text,
        isStreaming:
          isStreaming && i === message.parts.length - 1 && isLastMessage,
      })
      reasoningSegmentCount++
    } else if (part.type?.startsWith('tool-') || part.type === 'dynamic-tool') {
      const toolPart = part as {
        toolCallId: string
        type: string
        toolName?: string
        state: ToolInvocationState
        input: Record<string, unknown>
        output: unknown
        approval?: { id: string; approved?: boolean; reason?: string }
      }
      const toolName =
        part.type === 'dynamic-tool'
          ? (toolPart.toolName ?? 'tool')
          : toolPart.type.replace('tool-', '')

      if (HIDDEN_TOOLS.has(toolName)) continue

      currentToolBatch.push({
        state: toolPart.state,
        toolCallId: toolPart.toolCallId,
        toolName,
        input: toolPart?.input ?? {},
        output: toolPart?.output,
        approval: toolPart?.approval,
      })
    }
  }

  flushToolBatch()

  return segments
}
