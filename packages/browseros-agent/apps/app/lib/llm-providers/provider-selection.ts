import type { LlmProviderConfig } from './types'

/**
 * Resolves the persisted default id, repairing stale values to the first
 * provider.
 *
 * Null when there is nothing to point at, rather than an id naming a row that
 * does not exist.
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
