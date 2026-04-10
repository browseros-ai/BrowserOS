import { storage } from '@wxt-dev/storage'

export {
  extractPendingApprovals,
  queueApprovalResponse,
  removeApprovalResponsesById,
  removePendingApprovalsById,
  replacePendingApprovalsForConversation,
} from './approval-sync-helpers'

export interface PendingApproval {
  approvalId: string
  toolCallId: string
  toolName: string
  input: Record<string, unknown>
  conversationId: string
  timestamp: number
}

export interface ApprovalResponse {
  approvalId: string
  approved: boolean
  reason?: string
  timestamp: number
}

export interface ToolExecutionLogEntry {
  toolCallId: string
  toolName: string
  status: 'auto-allowed' | 'approved' | 'denied' | 'error'
  conversationId: string
  timestamp: number
  input?: Record<string, unknown>
}

export const pendingToolApprovalsStorage = storage.defineItem<
  PendingApproval[]
>('local:pending-tool-approvals', { fallback: [] })

export const approvalResponsesStorage = storage.defineItem<ApprovalResponse[]>(
  'local:approval-responses',
  { fallback: [] },
)

export const toolExecutionLogStorage = storage.defineItem<
  ToolExecutionLogEntry[]
>('local:tool-execution-log', { fallback: [] })
