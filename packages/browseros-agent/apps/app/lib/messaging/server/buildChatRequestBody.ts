import type { components } from '@browseros/server-api/openapi'
import { getModelsDevModels } from '@/lib/llm-providers/models-dev'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'

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

type Tab = components['schemas']['Tab']

// Chrome tabs may have no id. They cannot be addressed by the server.
type BrowserTab = Omit<Tab, 'id'> & { id?: number }

export interface ChatRequestBrowserContext {
  windowId?: number
  activeTab?: Partial<components['schemas']['Tab']>
  selectedTabs?: Partial<components['schemas']['Tab']>[]
}

export function toChatBrowserContext({
  windowId,
  activeTab,
  selectedTabs,
}: ChatRequestBrowserContext): components['schemas']['BrowserContext'] {
  const hasTabId = (tab: BrowserTab): tab is Tab => typeof tab.id === 'number'
  return {
    windowId,
    activeTab: activeTab && hasTabId(activeTab) ? activeTab : undefined,
    selectedTabs: selectedTabs?.filter(hasTabId),
  }
}

export interface ChatRequestBodyParams
  extends Omit<
    components['schemas']['ChatRequest'],
    'target' | 'browserContext' | 'supportsReasoning' | 'attachments'
  > {
  /**
   * The provider config, when the caller already holds it. Only the id is sent;
   * the rest is used to describe what the chosen model can do, which comes from
   * a catalogue the extension bundles.
   */
  provider?: LlmProviderConfig
  browserContext?: ChatRequestBrowserContext
}

type ProviderChatRequestBody = components['schemas']['ChatRequest'] & {
  historyMode: 'local' | 'cloud'
}

export const buildChatRequestBody = ({
  conversationId,
  userMessageId,
  provider,
  message = '',
  mode,
  browserContext,
  userSystemPrompt,
  userWorkingDir,
  previousConversation,
  persist,
  selectedText,
  selectedTextSource,
}: ChatRequestBodyParams): ProviderChatRequestBody => ({
  // The provider is named, not described. The server holds the list and which
  // one is selected, so it resolves the model, endpoint and credentials from
  // the id. Those used to travel on every message, which meant the api key and
  // the aws secret crossed the wire each time the user pressed send.
  target: {
    type: 'browseros',
    // Absent without a provider, which tells the server to use the selected
    // one.
    providerId: provider?.id,
  },
  message,
  conversationId,
  userMessageId,
  mode,
  browserContext: browserContext && toChatBrowserContext(browserContext),
  userSystemPrompt,
  userWorkingDir,
  supportsReasoning: provider ? resolvesSupportsReasoning(provider) : undefined,
  previousConversation,
  persist,
  // Bundled TypeScript servers through 0.0.166 read historyMode. TypeScript
  // 0.0.167 and later and Rust read persist. Drop historyMode when fallback
  // to bundles older than 0.0.167 is no longer supported.
  historyMode: persist === false ? 'cloud' : 'local',
  selectedText,
  selectedTextSource,
})
