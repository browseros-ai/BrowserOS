import type { UIMessage } from 'ai'
import {
  TOOL_NOT_RUN,
  TOOL_STOPPED,
} from '@/lib/messaging/server/tool-output-status'

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
  skippedStatus?: typeof TOOL_STOPPED | typeof TOOL_NOT_RUN
}

export type MessageSegment =
  | { type: 'text'; key: string; text: string }
  | { type: 'reasoning'; key: string; text: string; isStreaming: boolean }
  | { type: 'tool-batch'; key: string; tools: ToolInvocationInfo[] }

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
        errorText?: string
      }
      const toolName =
        part.type === 'dynamic-tool'
          ? (toolPart.toolName ?? 'tool')
          : toolPart.type.replace('tool-', '')

      // The stream and History seal skipped calls as output-error to keep the
      // SDK tool state terminal. Preserve their distinction for the batch UI.
      const { state, errorText } = toolPart
      currentToolBatch.push({
        state,
        toolCallId: toolPart.toolCallId,
        toolName,
        input: toolPart?.input ?? {},
        output: toolPart?.output,
        approval: toolPart?.approval,
        skippedStatus:
          state === 'output-error' &&
          (errorText === TOOL_STOPPED || errorText === TOOL_NOT_RUN)
            ? errorText
            : undefined,
      })
    }
  }

  flushToolBatch()

  return segments
}
