import { buildToolLabel } from '../../../lib/tool-labels'
import type { HarnessAgentHistoryPage } from '../agents/agent-harness-types'
import type {
  AgentHistoryPageResponse,
  BrowserOSChatHistoryItem,
  BrowserOSChatHistoryToolCall,
} from './claw-chat-types'

export function mapHarnessHistoryPage(
  page: HarnessAgentHistoryPage,
): AgentHistoryPageResponse {
  const items: BrowserOSChatHistoryItem[] = page.items.map((item, index) => {
    const toolCalls = item.toolCalls?.map(
      (tool): BrowserOSChatHistoryToolCall => {
        const input = asRecord(tool.input)
        const { label, subject } = buildToolLabel(tool.toolName, input)
        return {
          toolName: tool.toolName,
          label,
          status: tool.status,
          ...(tool.toolCallId ? { toolCallId: tool.toolCallId } : {}),
          ...(subject ? { subject } : {}),
          ...(tool.input !== undefined ? { input: tool.input } : {}),
          ...(tool.output !== undefined ? { output: tool.output } : {}),
          ...(tool.error ? { error: tool.error } : {}),
          ...(tool.durationMs != null ? { durationMs: tool.durationMs } : {}),
        }
      },
    )

    return {
      id: item.id,
      role: item.role,
      text: item.text,
      timestamp: item.createdAt,
      messageSeq: index + 1,
      sessionKey: 'main',
      source: 'user-chat',
      ...(item.reasoning ? { reasoning: item.reasoning } : {}),
      ...(toolCalls && toolCalls.length > 0 ? { toolCalls } : {}),
    }
  })
  const updatedAt =
    page.items.length > 0
      ? Math.max(...page.items.map((item) => item.createdAt))
      : Date.now()

  return {
    agentId: page.agentId,
    sessionKey: 'main',
    session: {
      key: 'main',
      updatedAt,
      sessionId: 'main',
      agentId: page.agentId,
      kind: 'agent-harness',
      source: 'user-chat',
    },
    items,
    page: {
      hasMore: false,
      limit: items.length,
    },
  }
}

function asRecord(value: unknown): Record<string, unknown> | undefined {
  return value && typeof value === 'object' && !Array.isArray(value)
    ? (value as Record<string, unknown>)
    : undefined
}
