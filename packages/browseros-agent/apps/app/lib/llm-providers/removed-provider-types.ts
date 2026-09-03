import type { LlmProviderConfig } from './types'

/**
 * Provider types that no longer exist. Storage migrations 4 and 5 strip these,
 * but the `browseros.providers` pref backup has no migration path, so a stale
 * backup can still be holding them.
 */
export const REMOVED_PROVIDER_TYPES = new Set([
  'remote-hermes',
  'claude-code',
  'codex',
  'acp-custom',
])

export function dropRemovedProviderConfigs(
  providers: LlmProviderConfig[] | null,
): LlmProviderConfig[] | null {
  if (!providers) return providers
  return providers.filter(
    (provider) => !REMOVED_PROVIDER_TYPES.has(String(provider.type)),
  )
}
