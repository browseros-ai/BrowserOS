import { afterEach, describe, expect, test } from 'bun:test'
import { mkdtempSync } from 'node:fs'
import { rm } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { closeDb, initializeDb } from '../../../src/lib/db'
import {
  dbScheduledJobRunStore,
  MAX_RUNS_PER_JOB,
} from '../../../src/lib/schedules/run-store'
import { dbScheduledJobStore } from '../../../src/lib/schedules/schedule-store'

const JOB_ID = 'job-1'
const RUN_ID = 'run-1'

function baseJob() {
  return {
    id: JOB_ID,
    name: 'Morning digest',
    query: 'summarise my inbox',
    scheduleType: 'daily' as const,
  }
}

function baseRun(overrides: Record<string, unknown> = {}) {
  return {
    id: RUN_ID,
    jobId: JOB_ID,
    status: 'completed' as const,
    startedAt: 1000,
    ...overrides,
  }
}

describe('dbScheduledJobRunStore', () => {
  const tempDirs: string[] = []

  afterEach(async () => {
    closeDb()
    await Promise.all(
      tempDirs.map((dir) => rm(dir, { recursive: true, force: true })),
    )
    tempDirs.length = 0
  })

  async function useTempDbWithJob() {
    const dir = mkdtempSync(join(tmpdir(), 'browseros-runs-test-'))
    tempDirs.push(dir)
    initializeDb({ dbPath: join(dir, 'db', 'browseros.sqlite') })
    await dbScheduledJobStore.upsert(baseJob())
  }

  test('round-trips a run including its tool calls', async () => {
    await useTempDbWithJob()
    const toolCalls = [
      {
        id: 'call-1',
        name: 'browser_navigate',
        input: { url: 'https://example.com' },
        output: { ok: true },
        timestamp: '2026-01-02T03:04:05.000Z',
      },
    ]

    await dbScheduledJobRunStore.upsert(baseRun({ toolCalls }))

    expect((await dbScheduledJobRunStore.get(RUN_ID))?.toolCalls).toEqual(
      toolCalls,
    )
  })

  // A run is written when it starts and again when it finishes, so the update
  // path is the ordinary one rather than an edge case.
  test('upsert moves a run from running to completed', async () => {
    await useTempDbWithJob()
    await dbScheduledJobRunStore.upsert(baseRun({ status: 'running' }))

    await dbScheduledJobRunStore.upsert(
      baseRun({ status: 'completed', completedAt: 2000, result: 'done' }),
    )

    const saved = await dbScheduledJobRunStore.get(RUN_ID)
    expect(saved).toMatchObject({
      status: 'completed',
      completedAt: 2000,
      result: 'done',
    })
    expect(await dbScheduledJobRunStore.list()).toHaveLength(1)
  })

  test('insertIfAbsent leaves an existing run untouched', async () => {
    await useTempDbWithJob()
    await dbScheduledJobRunStore.upsert(baseRun({ result: 'original' }))

    const saved = await dbScheduledJobRunStore.insertIfAbsent(
      baseRun({ result: 'stale import' }),
    )

    expect(saved).toBeNull()
    expect((await dbScheduledJobRunStore.get(RUN_ID))?.result).toBe('original')
  })

  // Cascade, unlike the job to provider reference which is set null. A run
  // whose job is gone means nothing, and deleting a job already removed its
  // runs before this table existed.
  test('deleting a job removes its runs', async () => {
    await useTempDbWithJob()
    await dbScheduledJobRunStore.upsert(baseRun())

    await dbScheduledJobStore.remove(JOB_ID)

    expect(await dbScheduledJobRunStore.list()).toEqual([])
  })

  test('lists newest first', async () => {
    await useTempDbWithJob()
    await dbScheduledJobRunStore.upsert(baseRun({ id: 'old', startedAt: 1000 }))
    await dbScheduledJobRunStore.upsert(baseRun({ id: 'new', startedAt: 3000 }))

    expect((await dbScheduledJobRunStore.list()).map((r) => r.id)).toEqual([
      'new',
      'old',
    ])
  })

  // The extension applied this cap while it owned the history, so keeping it
  // is preserving behaviour rather than adding a policy.
  test('prune keeps the newest runs of a job and drops the rest', async () => {
    await useTempDbWithJob()
    for (let i = 0; i < MAX_RUNS_PER_JOB + 5; i += 1) {
      await dbScheduledJobRunStore.upsert(
        baseRun({ id: `run-${i}`, startedAt: 1000 + i }),
      )
    }

    const dropped = await dbScheduledJobRunStore.prune(JOB_ID)

    expect(dropped).toBe(5)
    const remaining = await dbScheduledJobRunStore.list()
    expect(remaining).toHaveLength(MAX_RUNS_PER_JOB)
    expect(remaining[0].startedAt).toBe(1000 + MAX_RUNS_PER_JOB + 4)
  })

  test('prune leaves a job under the cap alone', async () => {
    await useTempDbWithJob()
    await dbScheduledJobRunStore.upsert(baseRun())

    expect(await dbScheduledJobRunStore.prune(JOB_ID)).toBe(0)
    expect(await dbScheduledJobRunStore.list()).toHaveLength(1)
  })

  test('prune only touches the job it was given', async () => {
    await useTempDbWithJob()
    await dbScheduledJobStore.upsert({ ...baseJob(), id: 'job-2' })
    await dbScheduledJobRunStore.upsert(
      baseRun({ id: 'other', jobId: 'job-2' }),
    )
    for (let i = 0; i < MAX_RUNS_PER_JOB + 2; i += 1) {
      await dbScheduledJobRunStore.upsert(
        baseRun({ id: `run-${i}`, startedAt: 1000 + i }),
      )
    }

    await dbScheduledJobRunStore.prune(JOB_ID)

    const ids = (await dbScheduledJobRunStore.list()).map((r) => r.id)
    expect(ids).toContain('other')
    expect(ids).toHaveLength(MAX_RUNS_PER_JOB + 1)
  })
})
