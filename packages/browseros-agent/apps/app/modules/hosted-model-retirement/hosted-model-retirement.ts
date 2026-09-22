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
  // Compared as a plain string: the type is gone from the union, and the
  // whole point is to recognise it in storage written before it was removed.
  return sources.some((providers) =>
    providers.some((provider) => String(provider.type) === 'browseros'),
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

  // Sequential, backup first, and not a Promise.all. Reading extension storage
  // can apply a pending migration, and that migration now strips the hosted
  // provider; the resulting write reaches the pref backup through the watcher
  // the background registers. Taking the backup before touching extension
  // storage means the migration cannot erase the evidence on its way past.
  const backup = await deps.loadBackupProviders()
  const stored = await deps.loadStoredProviders()
  if (!hasHostedModelEvidence([backup, stored])) return false

  await deps.mark()
  return true
}
