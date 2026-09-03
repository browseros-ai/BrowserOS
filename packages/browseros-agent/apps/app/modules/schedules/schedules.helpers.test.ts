import { describe, expect, it } from 'bun:test'
import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'
import {
  type ScheduledJobRow,
  type ScheduledJobRunRow,
  toScheduledJob,
  toScheduledJobPayload,
  toScheduledJobRun,
  toScheduledJobRunPayload,
} from './schedules.helpers'

const ISO = '2026-01-02T03:04:05.000Z'
const EPOCH = Date.parse(ISO)

function jobRow(overrides: Partial<ScheduledJobRow> = {}): ScheduledJobRow {
  return {
    id: 'job-1',
    name: 'Morning digest',
    query: 'summarise my inbox',
    scheduleType: 'daily',
    scheduleTime: '09:00',
    scheduleInterval: null,
    enabled: true,
    providerId: null,
    lastRunAt: null,
    createdAt: EPOCH,
    updatedAt: EPOCH,
    ...overrides,
  }
}

function runRow(
  overrides: Partial<ScheduledJobRunRow> = {},
): ScheduledJobRunRow {
  return {
    id: 'run-1',
    jobId: 'job-1',
    status: 'completed',
    startedAt: EPOCH,
    completedAt: null,
    result: null,
    finalResult: null,
    executionLog: null,
    toolCalls: null,
    error: null,
    ...overrides,
  }
}

describe('toScheduledJob', () => {
  // The database holds epoch integers; the extension has always held ISO
  // strings and every consumer parses them that way.
  it('converts epoch times back to ISO strings', () => {
    const job = toScheduledJob(jobRow({ lastRunAt: EPOCH }))

    expect(job.createdAt).toBe(ISO)
    expect(job.lastRunAt).toBe(ISO)
  })

  it('turns absent columns into undefined rather than null', () => {
    const job = toScheduledJob(jobRow())

    expect(job.lastRunAt).toBeUndefined()
    expect(job.providerId).toBeUndefined()
    expect(job.scheduleInterval).toBeUndefined()
  })
})

describe('toScheduledJobPayload', () => {
  it('leaves the id out of the body', () => {
    const job = toScheduledJob(jobRow())
    expect('id' in toScheduledJobPayload(job)).toBe(false)
  })

  it('converts ISO times to epoch', () => {
    const job = toScheduledJob(jobRow({ lastRunAt: EPOCH }))
    const payload = toScheduledJobPayload(job)

    expect(payload.lastRunAt).toBe(EPOCH)
    expect(payload.createdAt).toBe(EPOCH)
  })

  it('drops an unparseable time rather than sending NaN', () => {
    const job = { ...toScheduledJob(jobRow()), createdAt: 'whenever' }
    expect(toScheduledJobPayload(job as ScheduledJob).createdAt).toBeUndefined()
  })
})

describe('toScheduledJobRun', () => {
  it('keeps the tool call log intact', () => {
    const toolCalls = [
      {
        id: 'call-1',
        name: 'browser_navigate',
        input: { url: 'https://example.com' },
        timestamp: ISO,
      },
    ]

    expect(toScheduledJobRun(runRow({ toolCalls })).toolCalls).toEqual(
      toolCalls,
    )
  })

  it('converts start and completion times to ISO', () => {
    const run = toScheduledJobRun(runRow({ completedAt: EPOCH }))

    expect(run.startedAt).toBe(ISO)
    expect(run.completedAt).toBe(ISO)
  })

  it('leaves an unfinished run without a completion time', () => {
    expect(toScheduledJobRun(runRow()).completedAt).toBeUndefined()
  })
})

describe('toScheduledJobRunPayload', () => {
  // startedAt is required by the server, so it cannot be dropped the way an
  // optional field can when it fails to parse.
  it('falls back to now when the start time is unusable', () => {
    const run = { ...toScheduledJobRun(runRow()), startedAt: 'whenever' }
    const payload = toScheduledJobRunPayload(run as ScheduledJobRun)

    expect(typeof payload.startedAt).toBe('number')
    expect(Number.isNaN(payload.startedAt)).toBe(false)
  })

  it('round-trips a run through both conversions', () => {
    const run = toScheduledJobRun(runRow({ completedAt: EPOCH }))
    const payload = toScheduledJobRunPayload(run)

    expect(payload.startedAt).toBe(EPOCH)
    expect(payload.completedAt).toBe(EPOCH)
    expect(payload.jobId).toBe('job-1')
  })
})
