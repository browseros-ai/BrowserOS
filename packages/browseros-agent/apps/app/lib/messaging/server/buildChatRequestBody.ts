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
  /**
   * The provider config, when the caller already holds it. Only the id is sent;
   * the rest is used to describe what the chosen model can do, which comes from
   * a catalogue the extension bundles.
   *
   * Callers that hold nothing but an id, such as the scheduled runner, pass
   * `providerId` instead and let the server resolve the rest.
   */
  provider?: LlmProviderConfig
  providerId?: string
  message?: string
  mode?: ChatMode
  /** Explicit submitting surface; context tabs never grant panel ownership. */
  panelTabId?: number
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
  providerId,
  message = '',
  mode,
  browserContext,
  panelTabId,
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
  // The provider is named, not described. The server holds the list and which
  // one is selected, so it resolves the model, endpoint and credentials from
  // the id. Those used to travel on every message, which meant the api key and
  // the aws secret crossed the wire each time the user pressed send.
  target: {
    type: 'browseros' as const,
    // Absent when the caller has neither, which tells the server to use the
    // selected provider.
    providerId: provider?.id ?? providerId,
  },
  message,
  conversationId,
  mode,
  browserContext,
  panelTabId,
  userSystemPrompt,
  userWorkingDir,
  // Sent because the caller can override what the provider says, and because
  // the reasoning answer comes from a model catalogue the extension bundles.
  supportsImages: supportsImages ?? provider?.supportsImages,
  supportsReasoning: provider ? resolvesSupportsReasoning(provider) : undefined,
  previousConversation,
  historyMode,
  declinedApps: declinedApps?.length ? declinedApps : undefined,
  selectedText,
  selectedTextSource,
  isScheduledTask,
})
