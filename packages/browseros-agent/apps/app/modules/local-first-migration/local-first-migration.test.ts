import { describe, expect, it } from 'bun:test'
import type { LlmProviderConfig } from '@/lib/llm-providers/types'
import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'
import {
  type LocalFirstMigrationDeps,
  type RunsMigrationDeps,
  runLocalFirstMigration,
  runScheduledRunsMigration,
} from './local-first-migration'
import type {
  ProviderImport,
  ScheduledJobImport,
} from './local-first-migration.helpers'

function provider(
  overrides: Partial<LlmProviderConfig> = {},
): LlmProviderConfig {
  return {
    id: 'provider-1',
    type: 'openai',
    name: 'My OpenAI',
    modelId: 'gpt-5.5',
    supportsImages: true,
    contextWindow: 200000,
    temperature: 0.2,
    apiKey: 'sk-test',
    createdAt: 10,
    updatedAt: 20,
    ...overrides,
  }
}

function job(overrides: Partial<ScheduledJob> = {}): ScheduledJob {
  return {
    id: 'job-1',
    name: 'Morning digest',
    query: 'summarise my inbox',
    scheduleType: 'daily',
    enabled: true,
    createdAt: '2026-01-02T03:04:05.000Z',
    updatedAt: '2026-01-02T03:04:05.000Z',
    ...overrides,
  }
}

interface Harness {
  deps: LocalFirstMigrationDeps
  done: () => boolean
  importedProviders: ProviderImport[][]
  importedJobs: ScheduledJobImport[][]
}

function harness(overrides: Partial<LocalFirstMigrationDeps> = {}): Harness {
  let done = false
  const importedProviders: ProviderImport[][] = []
  const importedJobs: ScheduledJobImport[][] = []

  return {
    done: () => done,
    importedProviders,
    importedJobs,
    deps: {
      isDone: async () => done,
      markDone: async () => {
        done = true
      },
      loadStoredProviders: async () => [],
      loadBackupProviders: async () => [],
      loadScheduledJobs: async () => [],
      importProviders: async (providers) => {
        importedProviders.push(providers)
      },
      importScheduledJobs: async (jobs) => {
        importedJobs.push(jobs)
      },
      ...overrides,
    },
  }
}

describe('runLocalFirstMigration', () => {
  it('imports providers and jobs, then records that it ran', async () => {
    const h = harness({
      loadStoredProviders: async () => [provider()],
      loadScheduledJobs: async () => [job()],
    })

    const result = await runLocalFirstMigration(h.deps)

    expect(result).toEqual({
      ranMigration: true,
      providerCount: 1,
      jobCount: 1,
    })
    expect(h.importedProviders[0][0]).toMatchObject({
      id: 'provider-1',
      apiKey: 'sk-test',
    })
    expect(h.importedJobs[0][0]).toMatchObject({ id: 'job-1' })
    expect(h.done()).toBe(true)
  })

  // The whole point of the marker: providers the user has since deleted must
  // not come back on the next startup.
  it('does nothing once it has already run', async () => {
    const h = harness({
      isDone: async () => true,
      loadStoredProviders: async () => [provider()],
    })

    const result = await runLocalFirstMigration(h.deps)

    expect(result.ranMigration).toBe(false)
    expect(h.importedProviders).toHaveLength(0)
  })

  it('unions the pref backup with extension storage', async () => {
    const h = harness({
      loadStoredProviders: async () => [provider()],
      loadBackupProviders: async () => [provider({ id: 'from-backup' })],
    })

    await runLocalFirstMigration(h.deps)

    expect(h.importedProviders[0].map((p) => p.id)).toEqual([
      'provider-1',
      'from-backup',
    ])
  })

  it('marks itself done with nothing to import so it stops retrying', async () => {
    const h = harness()

    const result = await runLocalFirstMigration(h.deps)

    expect(result).toEqual({
      ranMigration: true,
      providerCount: 0,
      jobCount: 0,
    })
    expect(h.importedProviders).toHaveLength(0)
    expect(h.importedJobs).toHaveLength(0)
    expect(h.done()).toBe(true)
  })

  // The whole batch is one request, so an entry the server rejects would take
  // the valid providers down with it, block the jobs queued behind it, and
  // leave the marker unset to fail again on every startup.
  it('drops an unusable backup entry instead of failing the batch', async () => {
    const h = harness({
      loadStoredProviders: async () => [provider()],
      loadBackupProviders: async () =>
        [
          { id: 'stale', name: 'half a provider' },
          provider({ id: 'removed-type', type: 'remote-hermes' as never }),
        ] as never,
      loadScheduledJobs: async () => [job()],
    })

    const result = await runLocalFirstMigration(h.deps)

    expect(h.importedProviders[0].map((p) => p.id)).toEqual(['provider-1'])
    expect(h.importedJobs).toHaveLength(1)
    expect(result.ranMigration).toBe(true)
    expect(h.done()).toBe(true)
  })

  it('drops a job the server would reject without losing the rest', async () => {
    const h = harness({
      loadScheduledJobs: async () =>
        [job(), { id: 'broken', name: '', query: '' }] as never,
    })

    await runLocalFirstMigration(h.deps)

    expect(h.importedJobs[0].map((j) => j.id)).toEqual(['job-1'])
    expect(h.done()).toBe(true)
  })

  // Filtering runs before the merge, so an unusable stored entry cannot win
  // the id and take a perfectly good backup copy down with it.
  it('falls back to the backup copy when the stored one is unusable', async () => {
    const h = harness({
      loadStoredProviders: async () => [{ id: 'provider-1' }] as never,
      loadBackupProviders: async () => [provider({ name: 'From backup' })],
    })

    await runLocalFirstMigration(h.deps)

    expect(h.importedProviders[0]).toHaveLength(1)
    expect(h.importedProviders[0][0].name).toBe('From backup')
  })

  // A failed run must retry on the next startup, which is only safe because
  // the server inserts what is absent rather than replacing.
  it('leaves itself unmarked when the import fails', async () => {
    const h = harness({
      loadStoredProviders: async () => [provider()],
      importProviders: async () => {
        throw new Error('server not up')
      },
    })

    await expect(runLocalFirstMigration(h.deps)).rejects.toThrow(
      'server not up',
    )
    expect(h.done()).toBe(false)
  })

  it('does not mark itself done when the jobs import fails after providers landed', async () => {
    const h = harness({
      loadStoredProviders: async () => [provider()],
      loadScheduledJobs: async () => [job()],
      importScheduledJobs: async () => {
        throw new Error('server not up')
      },
    })

    await expect(runLocalFirstMigration(h.deps)).rejects.toThrow(
      'server not up',
    )
    expect(h.importedProviders).toHaveLength(1)
    expect(h.done()).toBe(false)
  })
})

function runsHarness(overrides: Partial<RunsMigrationDeps> = {}) {
  let done = false
  const imported: ScheduledJobRun[][] = []
  return {
    done: () => done,
    imported,
    deps: {
      isDone: async () => done,
      markDone: async () => {
        done = true
      },
      loadRuns: async () => [],
      importRuns: async (runs: ScheduledJobRun[]) => {
        imported.push(runs)
      },
      ...overrides,
    } as RunsMigrationDeps,
  }
}

function jobRun(overrides: Partial<ScheduledJobRun> = {}): ScheduledJobRun {
  return {
    id: 'run-1',
    jobId: 'job-1',
    status: 'completed',
    startedAt: '2026-01-02T03:04:05.000Z',
    ...overrides,
  }
}

describe('runScheduledRunsMigration', () => {
  it('imports run history and records that it ran', async () => {
    const h = runsHarness({ loadRuns: async () => [jobRun()] })

    const result = await runScheduledRunsMigration(h.deps)

    expect(result).toEqual({ ranMigration: true, runCount: 1 })
    expect(h.imported[0][0].id).toBe('run-1')
    expect(h.done()).toBe(true)
  })

  it('does nothing once it has already run', async () => {
    const h = runsHarness({
      isDone: async () => true,
      loadRuns: async () => [jobRun()],
    })

    expect((await runScheduledRunsMigration(h.deps)).ranMigration).toBe(false)
    expect(h.imported).toHaveLength(0)
  })

  it('marks itself done with nothing to import so it stops retrying', async () => {
    const h = runsHarness()

    expect(await runScheduledRunsMigration(h.deps)).toEqual({
      ranMigration: true,
      runCount: 0,
    })
    expect(h.imported).toHaveLength(0)
    expect(h.done()).toBe(true)
  })

  it('leaves itself unmarked when the import fails', async () => {
    const h = runsHarness({
      loadRuns: async () => [jobRun()],
      importRuns: async () => {
        throw new Error('server not up')
      },
    })

    await expect(runScheduledRunsMigration(h.deps)).rejects.toThrow(
      'server not up',
    )
    expect(h.done()).toBe(false)
  })
})
