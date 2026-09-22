import type { LlmProviderConfig } from './types'

export const DEFAULT_PROVIDER_ID = 'browseros'
export const DEFAULT_PROVIDER_NAME = 'BrowserOS'

/**
 * Resolves the persisted default id, repairing stale values to the first
 * provider.
 *
 * Null when there is nothing to point at. It used to fall back to the built-in
 * provider's id, which was safe only while that provider was always seeded;
 * with nothing seeded the same fallback would hand every caller an id naming a
 * row that does not exist.
 */
export function resolveDefaultProviderId(
  providers: LlmProviderConfig[],
  defaultProviderId: string | null | undefined,
): string | null {
  if (
    defaultProviderId &&
    providers.some((provider) => provider.id === defaultProviderId)
  ) {
    return defaultProviderId
  }
  return providers[0]?.id ?? null
}

/** Resolves the provider selected by the persisted default id. */
export function resolveSelectedProvider(
  providers: LlmProviderConfig[],
  defaultProviderId: string | null,
): LlmProviderConfig | null {
  return (
    providers.find((provider) => provider.id === defaultProviderId) ??
    providers[0] ??
    null
  )
}
