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

  it('rejects a daily job with no time', async () => {
    const routes = createScheduledJobRoutes(memoryStore())
    const response = await put(routes, {
      ...body,
      scheduleType: 'daily',
      scheduleTime: null,
    })
    expect(response.status).toBe(400)
  })

  it('rejects a daily job with a malformed time', async () => {
    const routes = createScheduledJobRoutes(memoryStore())
    for (const scheduleTime of ['9am', '24:00', '09:60', '9:5', '']) {
      const response = await put(routes, { ...body, scheduleTime })
      expect(response.status).toBe(400)
    }
  })

  it('accepts daily edge times', async () => {
    for (const scheduleTime of ['00:00', '23:59']) {
      const response = await put(createScheduledJobRoutes(memoryStore()), {
        ...body,
        scheduleTime,
      })
      expect(response.status).toBe(200)
    }
  })

  it('rejects an interval job with no interval', async () => {
    const routes = createScheduledJobRoutes(memoryStore())
    for (const scheduleType of ['hourly', 'minutes'] as const) {
      const response = await put(routes, {
        ...body,
        scheduleType,
        scheduleTime: null,
        scheduleInterval: null,
      })
      expect(response.status).toBe(400)
    }
  })

  it('rejects an out-of-range or non-integer interval', async () => {
    const routes = createScheduledJobRoutes(memoryStore())
    for (const scheduleInterval of [0, -1, 61, 1.5]) {
      const response = await put(routes, {
        ...body,
        scheduleType: 'minutes',
        scheduleTime: null,
        scheduleInterval,
      })
      expect(response.status).toBe(400)
    }
  })

  it('accepts a valid interval job and persists the interval', async () => {
    for (const [scheduleType, scheduleInterval] of [
      ['hourly', 6],
      ['minutes', 30],
    ] as const) {
      const { store, rows } = memoryStore()
      const response = await put(createScheduledJobRoutes({ store }), {
        ...body,
        scheduleType,
        scheduleTime: null,
        scheduleInterval,
      })
      expect(response.status).toBe(200)
      expect(rows.get(JOB_ID)?.scheduleInterval).toBe(scheduleInterval)
    }
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
})
