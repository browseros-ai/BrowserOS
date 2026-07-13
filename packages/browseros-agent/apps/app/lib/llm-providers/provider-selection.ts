export const DEFAULT_PROVIDER_ID = 'browseros'
export const DEFAULT_PROVIDER_NAME = 'Request Browser'

/** Returns the product-facing name for a provider, repairing stale legacy labels at render time. */
export function getProviderDisplayName(
  provider: {
    id?: string
    rowId?: string
    type?: string
    name: string
  },
): string {
  const providerId = provider.id ?? provider.rowId
  return providerId === DEFAULT_PROVIDER_ID && provider.type === 'browseros'
    ? DEFAULT_PROVIDER_NAME
    : provider.name
}

/** Resolves the persisted default id, repairing stale values to the first provider. */
export function resolveDefaultProviderId(
  providers: LlmProviderConfig[],
  defaultProviderId: string | null | undefined,
): string {
  if (
    defaultProviderId &&
    providers.some((provider) => provider.id === defaultProviderId)
  ) {
    return defaultProviderId
  }
  return providers[0]?.id ?? DEFAULT_PROVIDER_ID
}

/** Resolves the provider selected by the persisted default id. */
export function resolveSelectedProvider(
  providers: LlmProviderConfig[],
  defaultProviderId: string,
): LlmProviderConfig | null {
  return (
    providers.find((provider) => provider.id === defaultProviderId) ??
    providers[0] ??
    null
  )
}
