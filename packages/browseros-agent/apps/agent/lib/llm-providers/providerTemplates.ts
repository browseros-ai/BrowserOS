import { getModelsDevProvider } from './models-dev'
import type { ProviderType } from './types'

/**
 * Provider template for quick setup
 * @public
 */
export interface ProviderTemplate {
  id: ProviderType
  name: string
  defaultBaseUrl: string
  defaultModelId: string
  supportsImages: boolean
  contextWindow: number
  setupGuideUrl?: string
  apiKeyUrl?: string
}

function enrichTemplate(
  providerId: ProviderType,
  overrides: {
    defaultModelId: string
    defaultBaseUrl?: string
    apiKeyUrl?: string
    setupGuideUrl?: string
  },
): ProviderTemplate {
  const provider = getModelsDevProvider(providerId)
  const model = provider?.models.find((m) => m.id === overrides.defaultModelId)

  return {
    id: providerId,
    name: provider?.name ?? providerId,
    defaultBaseUrl: overrides.defaultBaseUrl ?? provider?.api ?? '',
    defaultModelId: overrides.defaultModelId,
    supportsImages: model?.supportsImages ?? true,
    contextWindow: model?.contextWindow ?? 128000,
    ...(overrides.apiKeyUrl && { apiKeyUrl: overrides.apiKeyUrl }),
    ...(overrides.setupGuideUrl && { setupGuideUrl: overrides.setupGuideUrl }),
  }
}

/**
 * Available provider templates for quick setup
 * @public
 */
export const providerTemplates: ProviderTemplate[] = [
  {
    id: 'chatgpt-pro',
    name: 'ChatGPT Plus/Pro',
    defaultBaseUrl: 'https://chatgpt.com/backend-api',
    defaultModelId: 'gpt-5.3-codex',
    supportsImages: true,
    contextWindow: 400000,
    setupGuideUrl: 'https://docs.browseros.com/features/chatgpt-pro-oauth',
  },
  {
    id: 'github-copilot',
    name: 'GitHub Copilot',
    defaultBaseUrl: 'https://api.githubcopilot.com',
    defaultModelId: 'gpt-5-mini',
    supportsImages: true,
    contextWindow: 128000,
    setupGuideUrl: 'https://docs.browseros.com/features/github-copilot-oauth',
  },
  {
    id: 'qwen-code',
    name: 'Qwen Code',
    defaultBaseUrl: 'https://portal.qwen.ai/v1',
    defaultModelId: 'coder-model',
    supportsImages: true,
    contextWindow: 1000000,
    setupGuideUrl: 'https://docs.browseros.com/features/qwen-code-oauth',
  },
  {
    id: 'moonshot',
    name: 'Moonshot AI',
    defaultBaseUrl: 'https://api.moonshot.ai/v1',
    defaultModelId: 'kimi-k2.5',
    supportsImages: true,
    contextWindow: 200000,
    apiKeyUrl: 'https://platform.moonshot.ai/console/api-keys',
    setupGuideUrl: 'https://platform.moonshot.ai/console/api-keys',
  },
  enrichTemplate('openai', {
    defaultModelId: 'gpt-5',
    apiKeyUrl: 'https://platform.openai.com/api-keys',
    setupGuideUrl:
      'https://docs.browseros.com/features/bring-your-own-llm#openai',
  }),
  {
    id: 'openai-compatible',
    name: 'OpenAI Compatible',
    defaultBaseUrl: '',
    defaultModelId: '',
    supportsImages: true,
    contextWindow: 128000,
  },
  enrichTemplate('anthropic', {
    defaultModelId: 'claude-sonnet-4-6',
    apiKeyUrl: 'https://console.anthropic.com/settings/keys',
    setupGuideUrl:
      'https://docs.browseros.com/features/bring-your-own-llm#claude',
  }),
  enrichTemplate('google', {
    defaultModelId: 'gemini-2.5-flash',
    apiKeyUrl: 'https://aistudio.google.com/app/apikey',
    setupGuideUrl:
      'https://docs.browseros.com/features/bring-your-own-llm#gemini',
  }),
  {
    id: 'ollama',
    name: 'Ollama',
    defaultBaseUrl: 'http://localhost:11434/v1',
    defaultModelId: 'llama3.2',
    supportsImages: false,
    contextWindow: 128000,
    setupGuideUrl:
      'https://docs.browseros.com/features/bring-your-own-llm#ollama',
  },
  enrichTemplate('openrouter', {
    defaultModelId: 'anthropic/claude-sonnet-4.5',
    apiKeyUrl: 'https://openrouter.ai/keys',
    setupGuideUrl:
      'https://docs.browseros.com/features/bring-your-own-llm#openrouter',
  }),
  enrichTemplate('lmstudio', {
    defaultModelId: 'openai/gpt-oss-20b',
    defaultBaseUrl: 'http://localhost:1234/v1',
    setupGuideUrl:
      'https://docs.browseros.com/features/bring-your-own-llm#lmstudio',
  }),
  enrichTemplate('azure', {
    defaultModelId: '',
    apiKeyUrl:
      'https://portal.azure.com/#view/Microsoft_Azure_ProjectOxford/CognitiveServicesHub/~/OpenAI',
  }),
  enrichTemplate('bedrock', {
    defaultModelId: 'anthropic.claude-sonnet-4-6',
    setupGuideUrl:
      'https://docs.aws.amazon.com/bedrock/latest/userguide/getting-started.html',
  }),
  enrichTemplate('minimax', {
    defaultModelId: 'MiniMax-M2.7',
    apiKeyUrl:
      'https://platform.minimax.io/user-center/basic-information/interface-key',
    setupGuideUrl: 'https://platform.minimax.io/docs/guides/models-intro',
  }),
]

export const MINIMAX_REGIONS = {
  chinese: {
    api: 'https://api.minimaxi.com/v1',
    apiKeyUrl:
      'https://platform.minimaxi.com/user-center/basic-information/interface-key',
    setupGuideUrl: 'https://platform.minimaxi.com/document',
  },
  international: {
    api: 'https://api.minimax.io/v1',
    apiKeyUrl:
      'https://platform.minimax.io/user-center/basic-information/interface-key',
    setupGuideUrl: 'https://platform.minimax.io/docs/guides/models-intro',
  },
} as const

export type MinimaxRegion = keyof typeof MINIMAX_REGIONS

/**
 * Provider type options for select dropdowns
 * @public
 */
export const providerTypeOptions: { value: ProviderType; label: string }[] = [
  { value: 'chatgpt-pro', label: 'ChatGPT Plus/Pro' },
  { value: 'github-copilot', label: 'GitHub Copilot' },
  { value: 'qwen-code', label: 'Qwen Code' },
  { value: 'moonshot', label: 'Moonshot AI' },
  { value: 'anthropic', label: 'Anthropic' },
  { value: 'openai', label: 'OpenAI' },
  { value: 'openai-compatible', label: 'OpenAI Compatible' },
  { value: 'google', label: 'Gemini' },
  { value: 'openrouter', label: 'OpenRouter' },
  { value: 'azure', label: 'Azure' },
  { value: 'ollama', label: 'Ollama' },
  { value: 'lmstudio', label: 'LM Studio' },
  { value: 'bedrock', label: 'AWS Bedrock' },
  { value: 'browseros', label: 'BrowserOS' },
  { value: 'minimax', label: 'MiniMax' },
]

/**
 * Get provider template by type
 * @public
 */
export const getProviderTemplate = (
  type: ProviderType,
): ProviderTemplate | undefined => {
  return providerTemplates.find((t) => t.id === type)
}

/**
 * Default base URLs for each provider type
 * Auto-fills when user selects a provider type
 */
export const DEFAULT_BASE_URLS: Record<ProviderType, string> = {
  'chatgpt-pro': 'https://chatgpt.com/backend-api',
  'github-copilot': 'https://api.githubcopilot.com',
  'qwen-code': 'https://portal.qwen.ai/v1',
  moonshot: 'https://api.moonshot.ai/v1',
  anthropic: 'https://api.anthropic.com/v1',
  openai: 'https://api.openai.com/v1',
  'openai-compatible': '',
  google: 'https://generativelanguage.googleapis.com/v1beta',
  openrouter: 'https://openrouter.ai/api/v1',
  azure: '',
  ollama: 'http://localhost:11434/v1',
  lmstudio: 'http://localhost:1234/v1',
  bedrock: '',
  browseros: '',
  minimax: MINIMAX_REGIONS.chinese.api,
}

/**
 * Get default base URL for a provider type
 * @public
 */
export const getDefaultBaseUrlForProviders = (type: ProviderType): string => {
  return DEFAULT_BASE_URLS[type] || ''
}
