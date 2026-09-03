import type { LlmProviderRoutes, ScheduledJobRoutes } from '@browseros/server'
import { storage } from '@wxt-dev/storage'
import { hc } from 'hono/client'
import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { BROWSEROS_PREFS } from '@/lib/browseros/prefs'
import { providersStorage } from '@/lib/llm-providers/storage'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import { scheduledJobStorage } from '@/lib/schedules/scheduleStorage'
import { resolveAgentServerUrlWithRetry } from '@/modules/browseros/agent-server-url.helpers'
import { runLocalFirstMigration } from './local-first-migration'
import {
  type ProviderImport,
  parseProviderBackup,
  type ScheduledJobImport,
} from './local-first-migration.helpers'

/**
 * Per profile, because extension storage is per profile. Losing it costs a
 * redundant import that inserts nothing, never a lost or overwritten row,
 * which is what insert-if-absent on the server buys.
 */
export const migrationDoneStorage = storage.defineItem<boolean>(
  'local:local-first-migration-done',
  { fallback: false },
)

async function loadBackupProviders(): Promise<LlmProviderConfig[]> {
  try {
    const pref = await getBrowserOSAdapter().getPref(BROWSEROS_PREFS.PROVIDERS)
    return parseProviderBackup(pref?.value)
  } catch {
    // No BrowserOS API, or no backup written yet. Extension storage still runs.
    return []
  }
}

async function importProviders(providers: ProviderImport[]): Promise<void> {
  const baseUrl = await resolveAgentServerUrlWithRetry()
  const client = hc<LlmProviderRoutes>(`${baseUrl}/llm-providers`)
  const response = await client.import.$post({ json: { providers } })
  if (!response.ok) {
    throw new Error(`Failed to import providers (${response.status})`)
  }
}

async function importScheduledJobs(jobs: ScheduledJobImport[]): Promise<void> {
  const baseUrl = await resolveAgentServerUrlWithRetry()
  const client = hc<ScheduledJobRoutes>(`${baseUrl}/scheduled-jobs`)
  const response = await client.import.$post({ json: { jobs } })
  if (!response.ok) {
    throw new Error(`Failed to import scheduled jobs (${response.status})`)
  }
}

/** Fire and forget from the background; a failure retries on next startup. */
export function startLocalFirstMigration(): void {
  void runLocalFirstMigration({
    isDone: () => migrationDoneStorage.getValue(),
    markDone: () => migrationDoneStorage.setValue(true),
    loadStoredProviders: async () => (await providersStorage.getValue()) ?? [],
    loadBackupProviders,
    loadScheduledJobs: async () => (await scheduledJobStorage.getValue()) ?? [],
    importProviders,
    importScheduledJobs,
  }).catch(() => null)
}
