import type { AclRule } from '@browseros/shared/types/acl'
import type { ChatMode } from '@/entrypoints/sidepanel/index/chatTypes'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ToolApprovalConfig } from '@/lib/tool-approvals/types'

export interface ApprovalResponseData {
  approvalId: string
  approved: boolean
  reason?: string
}

export interface ChatHistoryEntry {
  role: 'user' | 'assistant'
  content: string
}

export interface ChatRequestBrowserContext {
  windowId?: number
  activeTab?: {
    id?: number
    url?: string
    title?: string
  }
  selectedTabs?: {
    id?: number
    url?: string
    title?: string
  }[]
  enabledMcpServers?: string[]
  customMcpServers?: {
    name: string
    url?: string
  }[]
}

interface ChatRequestBodyParams {
  conversationId: string
  provider: LlmProviderConfig
  message?: string
  mode?: ChatMode
  browserContext?: ChatRequestBrowserContext
  userSystemPrompt?: string
  userWorkingDir?: string
  supportsImages?: boolean
  previousConversation?: ChatHistoryEntry[] | string
  declinedApps?: string[]
  aclRules?: AclRule[]
  selectedText?: string
  selectedTextSource?: {
    url: string
    title: string
  }
  toolApprovalConfig?: ToolApprovalConfig
  toolApprovalResponses?: ApprovalResponseData[]
  isScheduledTask?: boolean
}

export const toRequestToolApprovalConfig = (
  approvalConfig?: ToolApprovalConfig,
): ToolApprovalConfig | undefined => {
  if (!approvalConfig) return undefined
  return Object.values(approvalConfig.categories).some(Boolean)
    ? approvalConfig
    : undefined
}

export const buildChatRequestBody = ({
  conversationId,
  provider,
  message = '',
  mode,
  browserContext,
  userSystemPrompt,
  userWorkingDir,
  supportsImages,
  previousConversation,
  declinedApps,
  aclRules,
  selectedText,
  selectedTextSource,
  toolApprovalConfig,
  toolApprovalResponses,
  isScheduledTask,
}: ChatRequestBodyParams) => ({
  message,
  provider: provider.type,
  providerType: provider.type,
  providerName: provider.name,
  apiKey: provider.apiKey,
  baseUrl: provider.baseUrl,
  conversationId,
  model: provider.modelId ?? 'default',
  mode,
  contextWindowSize: provider.contextWindow,
  temperature: provider.temperature,
  resourceName: provider.resourceName,
  accessKeyId: provider.accessKeyId,
  secretAccessKey: provider.secretAccessKey,
  region: provider.region,
  sessionToken: provider.sessionToken,
  reasoningEffort: provider.reasoningEffort,
  reasoningSummary: provider.reasoningSummary,
  browserContext,
  userSystemPrompt,
  userWorkingDir,
  supportsImages: supportsImages ?? provider.supportsImages,
  previousConversation,
  declinedApps: declinedApps?.length ? declinedApps : undefined,
  aclRules: aclRules?.length ? aclRules : undefined,
  selectedText,
  selectedTextSource,
  toolApprovalConfig: toRequestToolApprovalConfig(toolApprovalConfig),
  toolApprovalResponses,
  isScheduledTask,
})
