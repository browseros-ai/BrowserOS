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
  output: unknown[]
  approval?: { id: string; approved?: boolean; reason?: string }
}

export type NudgeType = 'schedule_suggestion' | 'app_connection'

export interface NudgeData {
  type: NudgeType
  [key: string]: unknown
}

export type MessageSegment =
  | { type: 'text'; key: string; text: string }
  | { type: 'reasoning'; key: string; text: string; isStreaming: boolean }
  | { type: 'tool-batch'; key: string; tools: ToolInvocationInfo[] }
  | { type: 'nudge'; key: string; nudgeType: NudgeType; data: NudgeData }

const NUDGE_TOOLS = new Set(['suggest_schedule', 'suggest_app_connection'])

// Some local models (observed via LM Studio) leak raw Harmony-style channel
// scaffold tokens like "<|channel|>" or the malformed "<channel|>" into
// plain text/reasoning output instead of confining them to a structured
// field. Matches only bracket+pipe token shapes ("<|word|>", "<|word>",
// "<word|>") so real HTML-like text (e.g. "<div>") and markdown tables
// ("| a | b |") are never touched.
const SCAFFOLD_TOKEN_PATTERN = /<\|[a-zA-Z_]+\|?>|<[a-zA-Z_]+\|>/g

export function stripScaffoldTokens(text: string): string {
  return text
    .replace(SCAFFOLD_TOKEN_PATTERN, '')
    .replace(/\n{3,}/g, '\n\n')
    .trim()
}

function pushTextSegment(
  segments: MessageSegment[],
  messageId: string,
  index: number,
  rawText: string,
): number {
  const text = stripScaffoldTokens(rawText)
  if (!text) return index
  segments.push({ type: 'text', key: `${messageId}-text-${index}`, text })
  return index + 1
}

function pushReasoningSegment(
  segments: MessageSegment[],
  messageId: string,
  index: number,
  rawText: string,
  isPartStreaming: boolean,
): number {
  const text = stripScaffoldTokens(rawText)
  if (!text) return index
  segments.push({
    type: 'reasoning',
    key: `${messageId}-reasoning-${index}`,
    text,
    isStreaming: isPartStreaming,
  })
  return index + 1
}

function parseNudgeOutput(output: unknown): NudgeData | null {
  try {
    // output is { content: [{ type: "text", text: "JSON..." }], isError: false }
    const result = output as {
      content?: Array<{ type: string; text?: string }>
      isError?: boolean
    }
    if (result?.isError) return null

    const text = result?.content?.find((c) => c.type === 'text')?.text
    if (!text) return null

    const parsed = JSON.parse(text)
    if (
      parsed?.type === 'schedule_suggestion' ||
      parsed?.type === 'app_connection'
    ) {
      return parsed as NudgeData
    }
  } catch {
    // ignore parse errors
  }
  return null
}

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
      textSegmentCount = pushTextSegment(
        segments,
        message.id,
        textSegmentCount,
        part.text,
      )
    } else if (part.type === 'reasoning') {
      flushToolBatch()
      reasoningSegmentCount = pushReasoningSegment(
        segments,
        message.id,
        reasoningSegmentCount,
        part.text,
        isStreaming && i === message.parts.length - 1 && isLastMessage,
      )
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
      // Phantom acpx-ai-provider tool-input-* stream emitted under a fresh
      // blockId ("acpx-N") that never reconciles with the real tool-call
      // id. The translator emits a paired dynamic-tool part with the real
      // id and full input + output, so dropping the phantom keeps the UI
      // honest until upstream fixes the id mismatch.
      // See: https://github.com/DaniAkash/acpx/issues/37
      if (toolPart.toolCallId?.startsWith('acpx-')) {
        continue
      }
      const toolName =
        part.type === 'dynamic-tool'
          ? (toolPart.toolName ?? 'tool')
          : toolPart.type.replace('tool-', '')

      if (NUDGE_TOOLS.has(toolName) && toolPart.state === 'output-available') {
        flushToolBatch()
        const nudgeData = parseNudgeOutput(toolPart.output)
        if (nudgeData) {
          segments.push({
            type: 'nudge',
            key: `${message.id}-nudge-${toolPart.toolCallId}`,
            nudgeType: nudgeData.type,
            data: nudgeData,
          })
        }
      } else if (!NUDGE_TOOLS.has(toolName)) {
        currentToolBatch.push({
          state: toolPart.state,
          toolCallId: toolPart.toolCallId,
          toolName,
          input: toolPart?.input ?? {},
          output: (toolPart?.output as unknown[]) ?? [],
          approval: toolPart?.approval,
        })
      }
    }
  }

  flushToolBatch()

  return segments
}
