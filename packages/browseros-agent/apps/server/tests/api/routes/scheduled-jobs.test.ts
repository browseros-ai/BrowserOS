import { describe, expect, it } from 'bun:test'
import { createScheduledJobRoutes } from '../../../src/api/routes/scheduled-jobs'
import type { ScheduledJobRow } from '../../../src/lib/db/schema'
import type {
  ScheduledJobStore,
  ScheduledJobUpsert,
} from '../../../src/lib/schedules/schedule-store'

const JOB_ID = 'job-1'

function row(overrides: Partial<ScheduledJobRow> = {}): ScheduledJobRow {
  return {
    id: JOB_ID,
    profileId: null,
    name: 'Morning digest',
    query: 'summarise my inbox',
    scheduleType: 'daily',
    scheduleTime: '09:00',
    scheduleInterval: null,
    enabled: true,
    providerId: 'provider-1',
    lastRunAt: null,
    createdAt: 1,
    updatedAt: 1,
    ...overrides,
  }
}

function memoryStore(initial: ScheduledJobRow[] = []) {
  const rows = new Map(initial.map((r) => [r.id, r]))
  const store: ScheduledJobStore = {
    list: async () => [...rows.values()],
    get: async (id) => rows.get(id) ?? null,
    upsert: async (input: ScheduledJobUpsert) => {
      const existing = rows.get(input.id)
      const saved = {
        ...row(),
        ...input,
        createdAt: existing?.createdAt ?? input.createdAt ?? 100,
        updatedAt: 200,
      } as ScheduledJobRow
      rows.set(saved.id, saved)
      return saved
    },
    insertIfAbsent: async (input: ScheduledJobUpsert) => {
      if (rows.has(input.id)) return null
      return store.upsert(input)
    },
    remove: async (id) => rows.delete(id),
  }
  return { store, rows }
}

const body = {
  name: 'Morning digest',
  query: 'summarise my inbox',
  scheduleType: 'daily' as const,
  scheduleTime: '09:00',
  providerId: 'provider-1',
}

function put(
  routes: ReturnType<typeof createScheduledJobRoutes>,
  payload: unknown,
) {
  return routes.request(`/${JOB_ID}`, {
    method: 'PUT',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(payload),
  })
}

describe('scheduled job routes', () => {
  it('lists jobs', async () => {
    const routes = createScheduledJobRoutes(memoryStore([row()]))
    const response = await routes.request('/')
    expect(response.status).toBe(200)
    expect(await response.json()).toMatchObject({
      jobs: [{ id: JOB_ID, name: 'Morning digest' }],
    })
  })

  it('returns 404 for an unknown job', async () => {
    const routes = createScheduledJobRoutes(memoryStore())
    expect((await routes.request(`/${JOB_ID}`)).status).toBe(404)
  })

  it('creates a job under the id from the path', async () => {
    const { store, rows } = memoryStore()
    const response = await put(createScheduledJobRoutes({ store }), body)
    expect(response.status).toBe(200)
    expect(rows.get(JOB_ID)?.query).toBe('summarise my inbox')
  })

  it('is idempotent: putting the same id twice keeps one row', async () => {
    const { store, rows } = memoryStore()
    const routes = createScheduledJobRoutes({ store })
    await put(routes, body)
    await put(routes, body)
    expect(rows.size).toBe(1)
  })

  // The job keeps pointing at the provider it was created against, which is
  // the reference the migration has to preserve when both move together.
  it('preserves the provider reference', async () => {
    const { store, rows } = memoryStore()
    await put(createScheduledJobRoutes({ store }), body)
    expect(rows.get(JOB_ID)?.providerId).toBe('provider-1')
  })

  it('accepts a job with no provider attached', async () => {
    const { store, rows } = memoryStore()
    const response = await put(createScheduledJobRoutes({ store }), {
      ...body,
      providerId: null,
    })
    expect(response.status).toBe(200)
    expect(rows.get(JOB_ID)?.providerId).toBeNull()
  })

  it('rejects an unknown schedule type', async () => {
    const routes = createScheduledJobRoutes(memoryStore())
    const response = await put(routes, { ...body, scheduleType: 'weekly' })
    expect(response.status).toBe(400)
  })

  it('rejects a body missing the query', async () => {
    const routes = createScheduledJobRoutes(memoryStore())
    const response = await put(routes, { name: 'no query' })
    expect(response.status).toBe(400)
  })

  it('deletes a job', async () => {
    const { store, rows } = memoryStore([row()])
    const routes = createScheduledJobRoutes({ store })
    expect(
      (await routes.request(`/${JOB_ID}`, { method: 'DELETE' })).status,
    ).toBe(200)
    expect(rows.size).toBe(0)
  })

  it('returns 404 deleting an unknown job', async () => {
    const routes = createScheduledJobRoutes(memoryStore())
    expect(
      (await routes.request(`/${JOB_ID}`, { method: 'DELETE' })).status,
    ).toBe(404)
  })

  describe('import', () => {
    async function importJobs(
      routes: ReturnType<typeof createScheduledJobRoutes>,
      jobs: unknown[],
    ) {
      return routes.request('/import', {
        method: 'POST',
        headers: { 'content-type': 'application/json' },
        body: JSON.stringify({ jobs }),
      })
    }

    it('inserts a job that is not there yet', async () => {
      const { store, rows } = memoryStore()
      const routes = createScheduledJobRoutes({ store })
      const response = await importJobs(routes, [{ ...body, id: JOB_ID }])

      expect(response.status).toBe(200)
      expect(await response.json()).toEqual({ imported: [JOB_ID], skipped: [] })
      expect(rows.get(JOB_ID)).toMatchObject({ name: 'Morning digest' })
    })

    it('leaves an existing job untouched and reports it skipped', async () => {
      const { store, rows } = memoryStore([row({ name: 'Edited since' })])
      const routes = createScheduledJobRoutes({ store })
      const response = await importJobs(routes, [
        { ...body, id: JOB_ID, name: 'Stale copy' },
      ])

      expect(await response.json()).toEqual({ imported: [], skipped: [JOB_ID] })
      expect(rows.get(JOB_ID)?.name).toBe('Edited since')
    })

    it('rejects a job with no id', async () => {
      const routes = createScheduledJobRoutes(memoryStore())
      expect((await importJobs(routes, [body])).status).toBe(400)
    })
  })
})
