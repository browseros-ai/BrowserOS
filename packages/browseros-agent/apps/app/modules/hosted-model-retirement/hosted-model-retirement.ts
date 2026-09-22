import type { LlmProviderConfig } from '@/lib/llm-providers/types'

/**
 * Whether this profile ever ran on the BrowserOS-hosted provider.
 *
 * Someone who had a working chat yesterday needs to be told the built-in model
 * is gone; someone installing today needs to be told to connect one. Both have
 * an empty provider list, so the copy has to be chosen from something else.
 *
 * The evidence is local and survives the row being deleted from the server
 * database: the `browseros.providers` Chromium pref is per profile and has no
 * migration path, and extension storage is frozen at whatever it last held.
 */
export function hasHostedModelEvidence(
  sources: ReadonlyArray<readonly LlmProviderConfig[]>,
): boolean {
  return sources.some((providers) =>
    providers.some((provider) => provider.type === 'browseros'),
  )
}

export interface DetectHostedModelRetirementDeps {
  isMarked: () => Promise<boolean>
  mark: () => Promise<void>
  loadStoredProviders: () => Promise<LlmProviderConfig[]>
  loadBackupProviders: () => Promise<LlmProviderConfig[]>
}

/**
 * Records the flag once, from evidence that will not last forever.
 *
 * Recorded rather than re-derived on every read because the sources it reads
 * are legacy: extension storage is no longer written, and the pref backup is
 * only still there because nothing has cleaned it up. Deciding once means
 * those can be retired later without changing what the user is told.
 */
export async function detectHostedModelRetirement(
  deps: DetectHostedModelRetirementDeps,
): Promise<boolean> {
  if (await deps.isMarked()) return true

  const [stored, backup] = await Promise.all([
    deps.loadStoredProviders(),
    deps.loadBackupProviders(),
  ])
  if (!hasHostedModelEvidence([stored, backup])) return false

  await deps.mark()
  return true
}
