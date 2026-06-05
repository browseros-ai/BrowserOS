import type { UIMessage } from 'ai'

const RECENT_EXACT_MESSAGES = 6
const SUMMARY_MESSAGE_MAX_CHARS = 1_200
const SUMMARY_TOTAL_MAX_CHARS = 32_000
const RECENT_MESSAGE_MAX_CHARS = 2_500
const TOOL_PAYLOAD_MAX_CHARS = 1_000

export interface ChatContinuationResult {
  messages: UIMessage[]
  compactedMessageCount: number
}

export function buildChatContinuationMessages(input: {
  messages: UIMessage[]
  latestUserMessageId?: string
  reason?: string
}): ChatContinuationResult {
  const messages = input.messages.filter((message) =>
    uiMessageToTranscriptText(message).trim(),
  )
  const latestUserIndex = findLatestUserIndex(
    messages,
    input.latestUserMessageId,
  )

  if (latestUserIndex === -1) {
    return { messages, compactedMessageCount: 0 }
  }

  const latestUser = textOnlyMessage(messages[latestUserIndex], {
    maxChars: Number.POSITIVE_INFINITY,
  })
  const prior = messages.slice(0, latestUserIndex)
  const recentCount = Math.min(RECENT_EXACT_MESSAGES, prior.length)
  const older = prior.slice(0, prior.length - recentCount)
  const recent = prior.slice(prior.length - recentCount)

  const continuationNote: UIMessage = {
    id: crypto.randomUUID(),
    role: 'user',
    parts: [
      {
        type: 'text',
        text: [
          '<continuation_context>',
          'BrowserOS compacted earlier sidepanel chat history after the provider reported that the input exceeded the model context window.',
          input.reason
            ? `Provider signal: ${truncateText(input.reason, 500)}`
            : '',
          '',
          'Summary of compacted older history:',
          formatSummary(older),
          '',
          'Recent exact turns:',
          formatRecent(recent),
          '',
          'Use this continuation note as memory of the prior conversation. Keep working from this state without asking the user to restart or repeat context.',
          '</continuation_context>',
        ]
          .filter((line) => line !== '')
          .join('\n'),
      },
    ],
  }

  return {
    messages: [
      continuationNote,
      ...recent.map((message) =>
        textOnlyMessage(message, { maxChars: RECENT_MESSAGE_MAX_CHARS }),
      ),
      latestUser,
    ],
    compactedMessageCount: prior.length,
  }
}

function findLatestUserIndex(
  messages: UIMessage[],
  latestUserMessageId?: string,
): number {
  if (latestUserMessageId) {
    const byId = messages.findIndex(
      (message) =>
        message.id === latestUserMessageId && message.role === 'user',
    )
    if (byId !== -1) return byId
  }
  for (let i = messages.length - 1; i >= 0; i--) {
    if (messages[i].role === 'user') return i
  }
  return -1
}

function textOnlyMessage(
  message: UIMessage,
  options: { maxChars: number },
): UIMessage {
  return {
    id: message.id,
    role: message.role,
    parts: [
      {
        type: 'text',
        text: truncateText(
          uiMessageToTranscriptText(message),
          options.maxChars,
        ),
      },
    ],
  }
}

function formatSummary(messages: UIMessage[]): string {
  if (messages.length === 0) {
    return '- No older text was available to summarize.'
  }

  const lines: string[] = []
  let totalChars = 0
  for (const [index, message] of messages.entries()) {
    const text = singleLine(uiMessageToTranscriptText(message))
    if (!text) continue

    const line = `- ${roleLabel(message.role)}: ${truncateText(
      text,
      SUMMARY_MESSAGE_MAX_CHARS,
    )}`
    if (
      totalChars + line.length > SUMMARY_TOTAL_MAX_CHARS &&
      lines.length > 0
    ) {
      lines.push(
        `- [omitted ${messages.length - index} compacted messages to keep the continuation note bounded]`,
      )
      break
    }
    lines.push(line)
    totalChars += line.length + 1
  }

  return lines.length > 0
    ? lines.join('\n')
    : '- No older text was available to summarize.'
}

function formatRecent(messages: UIMessage[]): string {
  if (messages.length === 0) return '- No recent exact turns were available.'
  return messages
    .map((message) => {
      const text = truncateText(
        uiMessageToTranscriptText(message),
        RECENT_MESSAGE_MAX_CHARS,
      )
      return `[${roleLabel(message.role)}]\n${text}`
    })
    .join('\n\n')
}

function uiMessageToTranscriptText(message: UIMessage): string {
  return message.parts.map(partToText).filter(Boolean).join('\n').trim()
}

function partToText(part: UIMessage['parts'][number]): string {
  if (part.type === 'text') return part.text
  if (part.type === 'reasoning') return `Reasoning: ${part.text}`
  if (part.type === 'source-url') return `Source: ${part.url}`
  if (part.type === 'source-document') return `Source document: ${part.title}`
  if (part.type === 'file') return `[file: ${part.mediaType}]`
  if (part.type === 'dynamic-tool') {
    return toolPartToText('dynamic-tool', part)
  }
  if (part.type.startsWith('tool-')) {
    return toolPartToText(part.type.slice(5), part)
  }
  if (part.type.startsWith('data-')) return ''
  return `[${part.type}]`
}

function toolPartToText(toolName: string, part: unknown): string {
  const value = part as {
    state?: string
    input?: unknown
    output?: unknown
    errorText?: string
  }
  const lines = [`Tool ${toolName}${value.state ? ` (${value.state})` : ''}`]
  if (value.input !== undefined) {
    lines.push(
      `input: ${truncateText(safeJson(value.input), TOOL_PAYLOAD_MAX_CHARS)}`,
    )
  }
  if (value.output !== undefined) {
    lines.push(
      `output: ${truncateText(safeJson(value.output), TOOL_PAYLOAD_MAX_CHARS)}`,
    )
  }
  if (value.errorText) lines.push(`error: ${value.errorText}`)
  return lines.join('\n')
}

function roleLabel(role: UIMessage['role']): string {
  return role === 'user' ? 'User' : 'Assistant'
}

function truncateText(value: string, maxChars: number): string {
  if (value.length <= maxChars) return value
  return `${value.slice(0, maxChars)}\n[truncated ${value.length - maxChars} characters]`
}

function singleLine(value: string): string {
  return value.replace(/\s+/g, ' ').trim()
}

function safeJson(value: unknown): string {
  if (typeof value === 'string') return value
  try {
    return JSON.stringify(value)
  } catch {
    return String(value)
  }
}
