import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type { ScheduledJob } from '@/lib/schedules/scheduleTypes'
import type {
  ProviderImport,
  ScheduledJobImport,
} from './local-first-migration.helpers'
import {
  isImportableJob,
  isImportableProvider,
  mergeProviderSources,
  toProviderImport,
  toScheduledJobImport,
} from './local-first-migration.helpers'

export interface LocalFirstMigrationDeps {
  isDone: () => Promise<boolean>
  markDone: () => Promise<void>
  loadStoredProviders: () => Promise<LlmProviderConfig[]>
  loadBackupProviders: () => Promise<LlmProviderConfig[]>
  loadScheduledJobs: () => Promise<ScheduledJob[]>
  importProviders: (providers: ProviderImport[]) => Promise<void>
  importScheduledJobs: (jobs: ScheduledJobImport[]) => Promise<void>
}

export interface LocalFirstMigrationResult {
  ranMigration: boolean
  providerCount: number
  jobCount: number
}

const SKIPPED: LocalFirstMigrationResult = {
  ranMigration: false,
  providerCount: 0,
  jobCount: 0,
}

/**
 * Moves providers and scheduled jobs from extension storage into the server
 * database, once.
 *
 * Only local sources are read. The cloud is deliberately not one: its
 * scheduled jobs include every job deleted since the deletion queue lost its
 * only reader, and its providers never carried credentials, so they are
 * already handled by the incomplete-provider prompt in AI settings.
 *
 * The done marker is set only after both imports land. A failed run leaves it
 * unset and retries on the next startup, which is safe because the server side
 * inserts only what is absent.
 */
export async function runLocalFirstMigration(
  deps: LocalFirstMigrationDeps,
): Promise<LocalFirstMigrationResult> {
  if (await deps.isDone()) return SKIPPED

  const [stored, backup, jobs] = await Promise.all([
    deps.loadStoredProviders(),
    deps.loadBackupProviders(),
    deps.loadScheduledJobs(),
  ])

  // Filtering happens before the merge, not after: an unusable stored entry
  // would otherwise win the id and then be dropped, losing a provider whose
  // backup copy was perfectly good.
  const providers = mergeProviderSources(
    stored.filter(isImportableProvider),
    backup.filter(isImportableProvider),
  ).map(toProviderImport)
  const scheduledJobs = jobs.filter(isImportableJob).map(toScheduledJobImport)

  if (providers.length > 0) await deps.importProviders(providers)
  if (scheduledJobs.length > 0) await deps.importScheduledJobs(scheduledJobs)

  await deps.markDone()

  return {
    ranMigration: true,
    providerCount: providers.length,
    jobCount: scheduledJobs.length,
  }
}
