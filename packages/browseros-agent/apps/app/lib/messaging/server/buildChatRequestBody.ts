import { getModelsDevModels } from '@/lib/llm-providers/models-dev'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ChatMode } from '@/modules/chat/chat-types'

/**
 * Resolves whether the selected model supports reasoning from the models.dev
 * catalog. Unknown/custom models default to true so the server still attempts
 * reasoning (it is model-gated per provider for the cases that would error).
 */
function resolvesSupportsReasoning(provider: LlmProviderConfig): boolean {
  const model = getModelsDevModels(provider.type).find(
    (m) => m.id === provider.modelId,
  )
  return model?.supportsReasoning ?? true
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

export interface ChatRequestBodyParams {
  conversationId: string
  provider: LlmProviderConfig
  message?: string
  mode?: ChatMode
  browserContext?: ChatRequestBrowserContext
  userSystemPrompt?: string
  userWorkingDir?: string
  supportsImages?: boolean
  previousConversation?: ChatHistoryEntry[] | string
  historyMode?: 'local' | 'cloud'
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
  historyMode,
  declinedApps,
  selectedText,
  selectedTextSource,
  isScheduledTask,
}: ChatRequestBodyParams) => ({
  target: { type: 'browseros' as const, providerId: provider.id },
  message,
  provider: provider.type,
  providerId: provider.id,
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
  supportsReasoning: resolvesSupportsReasoning(provider),
  previousConversation,
  historyMode,
  declinedApps: declinedApps?.length ? declinedApps : undefined,
  selectedText,
  selectedTextSource,
  isScheduledTask,
})
