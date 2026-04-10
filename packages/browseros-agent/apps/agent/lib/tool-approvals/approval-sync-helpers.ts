import type { UIMessage } from 'ai'
import type { ApprovalResponse, PendingApproval } from './approval-sync-storage'

export function extractPendingApprovals(
  messages: UIMessage[],
  conversationId: string,
  timestamp = Date.now(),
): PendingApproval[] {
  const pending: PendingApproval[] = []

  for (const msg of messages) {
    for (const part of msg.parts) {
      const toolPart = part as {
        type?: string
        state?: string
        toolCallId?: string
        input?: Record<string, unknown>
        approval?: { id: string }
      }

      if (
        toolPart.state === 'approval-requested' &&
        toolPart.approval?.id &&
        toolPart.toolCallId
      ) {
        pending.push({
          approvalId: toolPart.approval.id,
          toolCallId: toolPart.toolCallId,
          toolName: (toolPart.type ?? '').replace('tool-', ''),
          input: toolPart.input ?? {},
          conversationId,
          timestamp,
        })
      }
    }
  }

  return pending
}

export function replacePendingApprovalsForConversation(
  existing: PendingApproval[],
  conversationId: string,
  next: PendingApproval[],
): PendingApproval[] {
  const existingByApprovalId = new Map(
    existing.map((item) => [item.approvalId, item]),
  )
  const preserved = next.map((item) => {
    const current = existingByApprovalId.get(item.approvalId)
    return current ? { ...item, timestamp: current.timestamp } : item
  })

  return [
    ...existing.filter((item) => item.conversationId !== conversationId),
    ...preserved,
  ]
}

export function queueApprovalResponse(
  existing: ApprovalResponse[],
  response: ApprovalResponse,
): ApprovalResponse[] {
  return [
    ...existing.filter((item) => item.approvalId !== response.approvalId),
    response,
  ]
}

export function removePendingApprovalsById(
  existing: PendingApproval[],
  approvalIds: string[],
): PendingApproval[] {
  const ids = new Set(approvalIds)
  return existing.filter((item) => !ids.has(item.approvalId))
}

export function removeApprovalResponsesById(
  existing: ApprovalResponse[],
  approvalIds: string[],
): ApprovalResponse[] {
  const ids = new Set(approvalIds)
  return existing.filter((item) => !ids.has(item.approvalId))
}
