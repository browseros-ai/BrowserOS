import { describe, expect, it } from 'bun:test'
import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'
import { shouldCatchUpScheduledJob } from './scheduledJobCatchUp'

const now = Date.parse('2026-09-21T15:00:00Z')

function job(partial: Partial<ScheduledJob> = {}): ScheduledJob {
  return {
    id: 'job-1',
    name: 'test job',
    query: 'do work',
    enabled: true,
    scheduleType: 'minutes',
    scheduleInterval: 5,
    createdAt: new Date(now - 24 * 60 * 60 * 1000).toISOString(),
    updatedAt: new Date(now).toISOString(),
    ...partial,
  }
}

function run(
  startedAt: string,
  partial: Partial<ScheduledJobRun> = {},
): ScheduledJobRun {
  return {
    id: crypto.randomUUID(),
    jobId: 'job-1',
    startedAt,
    status: 'completed',
    ...partial,
  }
}

describe('shouldCatchUpScheduledJob', () => {
  it('catches up minute and hourly jobs from their own cadence', () => {
    expect(
      shouldCatchUpScheduledJob(
        job({ scheduleType: 'minutes', scheduleInterval: 5 }),
        [run(new Date(now - 10 * 60 * 1000).toISOString())],
        now,
      ),
    ).toBe(true)
    expect(
      shouldCatchUpScheduledJob(
        job({ scheduleType: 'hourly', scheduleInterval: 1 }),
        [run(new Date(now - 2 * 60 * 60 * 1000).toISOString())],
        now,
      ),
    ).toBe(true)
  })

  it('does not catch up an interval job before its interval elapses', () => {
    expect(
      shouldCatchUpScheduledJob(
        job({ scheduleType: 'minutes', scheduleInterval: 5 }),
        [run(new Date(now - 60 * 1000).toISOString())],
        now,
      ),
    ).toBe(false)
  })

  it('uses lastRunAt when run history is unavailable', () => {
    expect(
      shouldCatchUpScheduledJob(
        job({
          scheduleType: 'hourly',
          scheduleInterval: 1,
          lastRunAt: new Date(now - 2 * 60 * 60 * 1000).toISOString(),
        }),
        [],
        now,
      ),
    ).toBe(true)
  })

  it('never starts a second run while one is running', () => {
    expect(
      shouldCatchUpScheduledJob(
        job(),
        [
          run(new Date(now - 10 * 60 * 1000).toISOString(), {
            status: 'running',
          }),
        ],
        now,
      ),
    ).toBe(false)
  })

  it('waits for the daily scheduled time', () => {
    expect(
      shouldCatchUpScheduledJob(
        job({ scheduleType: 'daily', scheduleTime: '23:30' }),
        [],
        now,
      ),
    ).toBe(false)
  })

  it('catches up a daily job that has not run since its scheduled time', () => {
    expect(
      shouldCatchUpScheduledJob(
        job({ scheduleType: 'daily', scheduleTime: '08:00' }),
        [run(new Date(now - 24 * 60 * 60 * 1000).toISOString())],
        now,
      ),
    ).toBe(true)
  })

  it('does not catch up a daily job that ran after its scheduled time', () => {
    expect(
      shouldCatchUpScheduledJob(
        job({ scheduleType: 'daily', scheduleTime: '08:00' }),
        [run(new Date(now - 60 * 60 * 1000).toISOString())],
        now,
      ),
    ).toBe(false)
  })

  it('ignores disabled jobs', () => {
    expect(shouldCatchUpScheduledJob(job({ enabled: false }), [], now)).toBe(
      false,
    )
  })
})
