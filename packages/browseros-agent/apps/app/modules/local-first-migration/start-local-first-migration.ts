import type { ProviderRoutes, ScheduledJobRoutes } from '@browseros/server'
import { storage } from '@wxt-dev/storage'
import { hc } from 'hono/client'
import { getBrowserOSAdapter } from '@/lib/browseros/adapter'
import { BROWSEROS_PREFS } from '@/lib/browseros/prefs'
import {
  defaultProviderIdStorage,
  providersStorage,
} from '@/lib/llm-providers/storage'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import {
  scheduledJobRunStorage,
  scheduledJobStorage,
} from '@/lib/schedules/scheduleStorage'
import { sentry } from '@/lib/sentry/sentry'
import { resolveAgentServerUrlWithRetry } from '@/modules/browseros/agent-server-url.helpers'
import { putDefaultProvider } from '@/modules/llm-providers/llm-providers.api'
import { importScheduledJobRuns } from '@/modules/schedules/schedules.api'
import {
  runDefaultProviderMigration,
  runLocalFirstMigration,
  runScheduledRunsMigration,
} from './local-first-migration'
import {
  type ProviderImport,
  parseProviderBackup,
  type ScheduledJobImport,
} from './local-first-migration.helpers'
import { waitForAgentServer } from './wait-for-agent-server'

/**
 * Per profile, because extension storage is per profile. Losing it costs a
 * redundant import that inserts nothing, never a lost or overwritten row,
 * which is what insert-if-absent on the server buys.
 */
export const migrationDoneStorage = storage.defineItem<boolean>(
  'local:local-first-migration-done',
  { fallback: false },
)

/**
 * Runs carry their own marker rather than reusing the one above.
 *
 * Reusing it would mean re-running the provider and job import for everyone
 * who has already migrated, and that import must never run twice: extension
 * storage is frozen now, so it would insert back anything the user has since
 * deleted through the new UI.
 */
export const runsMigrationDoneStorage = storage.defineItem<boolean>(
  'local:local-first-runs-migration-done',
  { fallback: false },
)

/** Its own marker too, for the reason on the one above. */
export const defaultMigrationDoneStorage = storage.defineItem<boolean>(
  'local:local-first-default-migration-done',
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
  const client = hc<ProviderRoutes>(`${baseUrl}/providers`)
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

/**
 * Runs the one-time imports once the server is up.
 *
 * Everything here happens when the background starts, which is also when the
 * server starts, so firing straight away meant importing into a socket nothing
 * was listening on. Each import then failed, and because a failed run leaves
 * its marker unset it lost the same race on the next launch too, so a user
 * upgrading saw their providers and scheduled tasks simply never arrive while
 * the database migration looked like it had worked.
 *
 * Waiting on health first removes the race. A failure past that point is worth
 * seeing rather than swallowing: it is the difference between a slow start and
 * data that never came across.
 */
export function startLocalFirstMigration(): void {
  void (async () => {
    if (!(await waitForAgentServer())) {
      sentry.captureException(
        new Error('Agent server unreachable before the local-first import'),
        {
          extra: {
            message:
              'Imports deferred to the next start; markers remain unset so they will run again',
          },
        },
      )
      return
    }

    // The default is chained onto the provider import rather than run
    // alongside it: the id it names has to exist server side before it can be
    // made default. The run history is independent and does not wait.
    try {
      await runLocalFirstMigration({
        isDone: () => migrationDoneStorage.getValue(),
        markDone: () => migrationDoneStorage.setValue(true),
        loadStoredProviders: async () =>
          (await providersStorage.getValue()) ?? [],
        loadBackupProviders,
        loadScheduledJobs: async () =>
          (await scheduledJobStorage.getValue()) ?? [],
        importProviders,
        importScheduledJobs,
      })
      await runDefaultProviderMigration({
        isDone: () => defaultMigrationDoneStorage.getValue(),
        markDone: () => defaultMigrationDoneStorage.setValue(true),
        loadStoredDefaultId: async () =>
          (await defaultProviderIdStorage.getValue()) || null,
        setDefault: putDefaultProvider,
      })
    } catch (error) {
      // Reported rather than swallowed: a silent failure here is
      // indistinguishable from the user's data having vanished, which is
      // exactly how this went unnoticed.
      sentry.captureException(error, {
        extra: { message: 'Provider and scheduled job import failed' },
      })
    }

    try {
      await runScheduledRunsMigration({
        isDone: () => runsMigrationDoneStorage.getValue(),
        markDone: () => runsMigrationDoneStorage.setValue(true),
        loadRuns: async () => (await scheduledJobRunStorage.getValue()) ?? [],
        importRuns: importScheduledJobRuns,
      })
    } catch (error) {
      sentry.captureException(error, {
        extra: { message: 'Scheduled run history import failed' },
      })
    }
  })()
}
