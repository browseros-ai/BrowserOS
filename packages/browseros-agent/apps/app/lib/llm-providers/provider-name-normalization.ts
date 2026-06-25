import { CHATGPT_PROVIDER_DISPLAY_NAME } from './provider-display-names'
import {
  DEFAULT_PROVIDER_ID,
  DEFAULT_PROVIDER_NAME,
} from './provider-selection'
import type { LlmProviderConfig } from './types'

/** Applies the v3 provider display-name compatibility migration. */
export function migrateLlmProvidersToV3(
  providers: LlmProviderConfig[] | null,
): LlmProviderConfig[] | null {
  if (!providers) return providers
  return normalizeProviderNames(providers)
}

/** Applies compatibility renames for stored provider display names. */
export function normalizeProviderNames(
  providers: LlmProviderConfig[],
): LlmProviderConfig[] {
  return providers.map((provider) => {
    if (
      provider.id === DEFAULT_PROVIDER_ID &&
      provider.type === 'browseros' &&
      provider.name !== DEFAULT_PROVIDER_NAME
    ) {
      return {
        ...provider,
        name: DEFAULT_PROVIDER_NAME,
      }
    }
    if (
      provider.type === 'chatgpt-pro' &&
      isLegacyChatGPTProviderName(provider.name)
    ) {
      return {
        ...provider,
        name: CHATGPT_PROVIDER_DISPLAY_NAME,
      }
    }
    return provider
  })
}

function isLegacyChatGPTProviderName(name: string): boolean {
  return /^ChatGPT Plus\/Pro(?: \([^@\s()]+@[^@\s()]+\))?$/.test(name)
}
