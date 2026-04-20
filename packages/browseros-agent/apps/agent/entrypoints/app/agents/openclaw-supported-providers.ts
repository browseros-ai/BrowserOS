import type { LlmProviderConfig, ProviderType } from '@/lib/llm-providers/types'

const OPENCLAW_SUPPORTED_PROVIDER_TYPES: ProviderType[] = [
  'openrouter',
  'openai',
  'openai-compatible',
  'anthropic',
  'moonshot',
]

export function isOpenClawSupportedProviderType(
  providerType: ProviderType,
): boolean {
  return OPENCLAW_SUPPORTED_PROVIDER_TYPES.includes(providerType)
}

export function getOpenClawSupportedProviders(
  providers: LlmProviderConfig[],
): LlmProviderConfig[] {
  return providers.filter(
    (provider) =>
      !!provider.apiKey && isOpenClawSupportedProviderType(provider.type),
  )
}
