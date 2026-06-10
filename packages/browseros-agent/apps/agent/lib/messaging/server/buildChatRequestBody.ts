import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ChatMode } from '@/modules/chat/chat-types'

interface ChatHistoryEntry {
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
  selectedText?: string
  selectedTextSource?: {
    url: string
    title: string
  }
  isScheduledTask?: boolean
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
  selectedText,
  selectedTextSource,
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
  selectedText,
  selectedTextSource,
  isScheduledTask,
})
