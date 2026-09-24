import type { LlmProviderConfig } from './types'

/**
 * Provider types that no longer exist. Storage migrations 4 and 5 strip these,
 * but the `browseros.providers` pref backup has no migration path, so a stale
 * backup can still be holding them.
 *
 * `browseros` is here for the same reason it is deleted from the database: the
 * hosted provider is retired, and the one-time import would otherwise put it
 * straight back from a backup written before the upgrade.
 */
export const REMOVED_PROVIDER_TYPES = new Set([
  'remote-hermes',
  'claude-code',
  'codex',
  'acp-custom',
  'browseros',
])

export function dropRemovedProviderConfigs(
  providers: LlmProviderConfig[] | null,
): LlmProviderConfig[] | null {
  if (!providers) return providers
  return providers.filter(
    (provider) => !REMOVED_PROVIDER_TYPES.has(String(provider.type)),
  )
}
