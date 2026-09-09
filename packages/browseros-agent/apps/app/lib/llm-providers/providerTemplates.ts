import { LLMProviderSchema } from '@browseros/shared/schemas/llm'
import { getModelsDevProvider } from './models-dev'
import { CHATGPT_PROVIDER_DISPLAY_NAME } from './provider-display-names'
import type { ProviderType } from './types'

/**
 * Provider template for quick setup
 * @public
 */
export interface ProviderTemplate {
  id: ProviderType
  name: string
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
    apiKeyUrl?: string
    setupGuideUrl?: string
  },
): ProviderTemplate {
  const provider = getModelsDevProvider(providerId)
  const model = provider?.models.find((m) => m.id === overrides.defaultModelId)

  return {
    id: providerId,
    name: provider?.name ?? providerId,
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
    name: CHATGPT_PROVIDER_DISPLAY_NAME,
    defaultModelId: 'gpt-5.5',
    supportsImages: true,
    contextWindow: 1050000,
    setupGuideUrl: 'https://docs.browseros.com/features/chatgpt-pro-oauth',
  },
  {
    id: 'github-copilot',
    name: 'GitHub Copilot',
    defaultModelId: 'gpt-5-mini',
    supportsImages: true,
    contextWindow: 128000,
    setupGuideUrl: 'https://docs.browseros.com/features/github-copilot-oauth',
  },
  {
    id: 'qwen-code',
    name: 'Qwen Code',
    defaultModelId: 'coder-model',
    supportsImages: true,
    contextWindow: 1000000,
    setupGuideUrl: 'https://docs.browseros.com/features/qwen-code-oauth',
  },
  {
    id: 'moonshot',
    name: 'Moonshot AI',
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
]

/**
 * Provider type options for select dropdowns
 * @public
 */
export const providerTypeOptions: { value: ProviderType; label: string }[] = [
  { value: 'chatgpt-pro', label: CHATGPT_PROVIDER_DISPLAY_NAME },
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

/** Recognizes provider types supported by this build without choosing their defaults. */
export function isProviderType(value: string): value is ProviderType {
  return LLMProviderSchema.safeParse(value).success
}
