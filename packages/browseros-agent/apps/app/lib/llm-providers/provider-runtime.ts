import type { LlmProviderConfig, ProviderType } from './types'

export function isChatProviderType(_type: ProviderType): boolean {
  return true
}

export function findChatProviderById(
  providers: LlmProviderConfig[],
  providerId?: string | null,
): LlmProviderConfig | null {
  if (!providerId) return null
  return providers.find((provider) => provider.id === providerId) ?? null
}

export function canTestProvider(_provider: LlmProviderConfig): boolean {
  return true
}

export function resolveChatProvider(
  providers: LlmProviderConfig[],
  preferredProviderId?: string | null,
): LlmProviderConfig | null {
  if (preferredProviderId) {
    const preferred = findChatProviderById(providers, preferredProviderId)
    if (preferred) return preferred
  }
  return providers[0] ?? null
}

export const findCloudChatProviderById = findChatProviderById
export const resolveCloudChatProvider = resolveChatProvider
