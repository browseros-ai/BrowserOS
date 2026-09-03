import { describe, expect, it } from 'bun:test'
import { createScheduledJobRunRoutes } from '../../../src/api/routes/scheduled-job-runs'
import type { ScheduledJobRunRow } from '../../../src/lib/db/schema'
import type {
  ScheduledJobRunStore,
  ScheduledJobRunUpsert,
} from '../../../src/lib/schedules/run-store'

const RUN_ID = 'run-1'

function row(overrides: Partial<ScheduledJobRunRow> = {}): ScheduledJobRunRow {
  return {
    id: RUN_ID,
    profileId: null,
    jobId: 'job-1',
    status: 'completed',
    startedAt: 1000,
    completedAt: 2000,
    result: 'done',
    finalResult: null,
    executionLog: null,
    toolCalls: null,
    error: null,
    createdAt: 1,
    updatedAt: 2,
    ...overrides,
  }
}

function memoryStore(initial: ScheduledJobRunRow[] = []) {
  const rows = new Map(initial.map((r) => [r.id, r]))
  const store: ScheduledJobRunStore = {
    list: async () => [...rows.values()],
    get: async (id) => rows.get(id) ?? null,
    upsert: async (input: ScheduledJobRunUpsert) => {
      const existing = rows.get(input.id)
      const saved = {
        ...row(),
        ...input,
        createdAt: existing?.createdAt ?? input.createdAt ?? 100,
        updatedAt: 200,
      } as ScheduledJobRunRow
      rows.set(saved.id, saved)
      return saved
    },
    insertIfAbsent: async (input: ScheduledJobRunUpsert) => {
      if (rows.has(input.id)) return null
      return store.upsert(input)
    },
    remove: async (id) => rows.delete(id),
    prune: async (jobId, keep = 15) => {
      const ofJob = [...rows.values()]
        .filter((r) => r.jobId === jobId)
        .sort((a, b) => b.startedAt - a.startedAt)
      const stale = ofJob.slice(keep)
      for (const run of stale) rows.delete(run.id)
      return stale.length
    },
  }
  return { store, rows }
}

const body = {
  jobId: 'job-1',
  status: 'running' as const,
  startedAt: 1000,
}

function put(
  routes: ReturnType<typeof createScheduledJobRunRoutes>,
  payload: unknown,
  runId = RUN_ID,
) {
  return routes.request(`/${runId}`, {
    method: 'PUT',
    headers: { 'content-type': 'application/json' },
    body: JSON.stringify(payload),
  })
}

describe('scheduled job run routes', () => {
  it('lists runs', async () => {
    const routes = createScheduledJobRunRoutes(memoryStore([row()]))
    const response = await routes.request('/')

    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({ runs: [{ id: RUN_ID }] })
  })

  it('writes a run', async () => {
    const { store, rows } = memoryStore()
    const response = await put(createScheduledJobRunRoutes({ store }), body)

    expect(response.status).toBe(200)
    expect(rows.get(RUN_ID)).toMatchObject({ status: 'running' })
  })

  it('keeps the tool call log across a write', async () => {
    const { store, rows } = memoryStore()
    const toolCalls = [
      {
        id: 'call-1',
        name: 'browser_navigate',
        input: { url: 'https://example.com' },
        timestamp: '2026-01-02T03:04:05.000Z',
      },
    ]

    await put(createScheduledJobRunRoutes({ store }), { ...body, toolCalls })

    expect(rows.get(RUN_ID)?.toolCalls).toEqual(toolCalls)
  })

  // The cap moved here from the extension, so a write has to apply it or the
  // history grows without bound now that nothing else trims it.
  it('trims a job past the run cap on write', async () => {
    const existing = Array.from({ length: 15 }, (_, i) =>
      row({ id: `run-${i}`, startedAt: 1000 + i }),
    )
    const { store, rows } = memoryStore(existing)

    await put(
      createScheduledJobRunRoutes({ store }),
      { ...body, startedAt: 9999 },
      'run-new',
    )

    expect(rows.size).toBe(15)
    expect(rows.has('run-0')).toBe(false)
    expect(rows.has('run-new')).toBe(true)
  })

  it('rejects a status the schema does not know', async () => {
    const routes = createScheduledJobRunRoutes(memoryStore())
    const response = await put(routes, { ...body, status: 'cancelled' })

    expect(response.status).toBe(400)
  })

  it('returns 404 for an unknown run', async () => {
    const routes = createScheduledJobRunRoutes(memoryStore())
    expect((await routes.request(`/${RUN_ID}`)).status).toBe(404)
  })

  it('deletes a run', async () => {
    const { store, rows } = memoryStore([row()])
    const routes = createScheduledJobRunRoutes({ store })

    expect(
      (await routes.request(`/${RUN_ID}`, { method: 'DELETE' })).status,
    ).toBe(200)
    expect(rows.size).toBe(0)
  })

  describe('import', () => {
    async function importRuns(
      routes: ReturnType<typeof createScheduledJobRunRoutes>,
      runs: unknown[],
    ) {
      return routes.request('/import', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ runs }),
      })
    }

    it('inserts a run that is not there yet', async () => {
      const { store, rows } = memoryStore()
      const response = await importRuns(
        createScheduledJobRunRoutes({ store }),
        [{ ...body, id: RUN_ID }],
      )

      expect(await response.json()).toEqual({ imported: [RUN_ID], skipped: [] })
      expect(rows.size).toBe(1)
    })

    it('leaves an existing run untouched and reports it skipped', async () => {
      const { store, rows } = memoryStore([row({ result: 'original' })])
      const response = await importRuns(
        createScheduledJobRunRoutes({ store }),
        [{ ...body, id: RUN_ID, result: 'stale import' }],
      )

      expect(await response.json()).toEqual({ imported: [], skipped: [RUN_ID] })
      expect(rows.get(RUN_ID)?.result).toBe('original')
    })

    // The list route is /, so a run whose id is "import" would otherwise be
    // reachable at the same path as the import endpoint.
    it('does not treat the import path as a run id', async () => {
      const { store } = memoryStore()
      const response = await importRuns(
        createScheduledJobRunRoutes({ store }),
        [],
      )

      expect(response.status).toBe(200)
      expect(await response.json()).toEqual({ imported: [], skipped: [] })
    })
  })
})
