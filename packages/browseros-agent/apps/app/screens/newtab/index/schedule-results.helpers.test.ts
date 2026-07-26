import { describe, expect, it } from 'bun:test'
import type {
  ScheduledJob,
  ScheduledJobRun,
} from '@/lib/schedules/scheduleTypes'
import {
  countRunningRuns,
  selectDisplayedRuns,
} from './schedule-results.helpers'

const job = (id: string, name: string): ScheduledJob => ({
  id,
  name,
  query: 'q',
  scheduleType: 'daily',
  enabled: true,
  createdAt: '2026-01-01T00:00:00.000Z',
  updatedAt: '2026-01-01T00:00:00.000Z',
})

const run = (
  id: string,
  jobId: string,
  status: ScheduledJobRun['status'],
  startedAt: string,
): ScheduledJobRun => ({ id, jobId, status, startedAt })

const jobs = [job('j1', 'Daily digest'), job('j2', 'Hourly check')]

describe('selectDisplayedRuns', () => {
  it('lists running runs first, then most-recent finished, capped at max', () => {
    const runs = [
      run('r1', 'j1', 'completed', '2026-06-10T10:00:00.000Z'),
      run('r2', 'j2', 'running', '2026-06-10T09:00:00.000Z'),
      run('r3', 'j1', 'failed', '2026-06-11T10:00:00.000Z'),
      run('r4', 'j2', 'completed', '2026-06-09T10:00:00.000Z'),
    ]

    const result = selectDisplayedRuns(runs, jobs, 3)

    expect(result.map((r) => r.id)).toEqual(['r2', 'r3', 'r1'])
    expect(result[0].job?.name).toBe('Hourly check')
  })

  it('keeps every running run when they meet or exceed max', () => {
    const runs = [
      run('r1', 'j1', 'running', '2026-06-10T10:00:00.000Z'),
      run('r2', 'j1', 'running', '2026-06-10T11:00:00.000Z'),
      run('r3', 'j2', 'running', '2026-06-10T12:00:00.000Z'),
      run('r4', 'j2', 'running', '2026-06-10T13:00:00.000Z'),
      run('r5', 'j1', 'completed', '2026-06-12T10:00:00.000Z'),
    ]

    const result = selectDisplayedRuns(runs, jobs, 3)

    expect(result).toHaveLength(4)
    expect(result.every((r) => r.status === 'running')).toBe(true)
  })

  it('pairs a run with undefined job when none matches', () => {
    const runs = [run('r1', 'missing', 'completed', '2026-06-10T10:00:00.000Z')]

    expect(selectDisplayedRuns(runs, jobs, 3)[0].job).toBeUndefined()
  })
})

describe('countRunningRuns', () => {
  it('counts only running runs', () => {
    const runs = [
      run('r1', 'j1', 'running', '2026-06-10T10:00:00.000Z'),
      run('r2', 'j1', 'completed', '2026-06-10T10:00:00.000Z'),
      run('r3', 'j1', 'running', '2026-06-10T10:00:00.000Z'),
    ]

    expect(countRunningRuns(runs)).toBe(2)
  })
})
