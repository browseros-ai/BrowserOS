import { describe, expect, it } from 'bun:test'
import { buildScheduledTaskResultChatContext } from './scheduledChatContext'
import type { ScheduledJob, ScheduledJobRun } from './scheduleTypes'

const baseRun: ScheduledJobRun = {
  id: 'run-1',
  jobId: 'job-1',
  startedAt: '2026-05-09T14:00:00.000Z',
  completedAt: '2026-05-09T14:03:00.000Z',
  status: 'completed',
  result: 'Headline one\nHeadline two',
}

const baseJob: ScheduledJob = {
  id: 'job-1',
  name: 'Morning News',
  query: 'Collect the latest important headlines.',
  scheduleType: 'daily',
  scheduleTime: '09:00',
  enabled: true,
  createdAt: '2026-05-01T00:00:00.000Z',
  updatedAt: '2026-05-01T00:00:00.000Z',
}

describe('buildScheduledTaskResultChatContext', () => {
  it('labels scheduled run output as background context', () => {
    const context = buildScheduledTaskResultChatContext({
      run: baseRun,
      job: baseJob,
    })

    expect(context).toContain('Scheduled task result context')
    expect(context).toContain(
      'This is background context, not a new user instruction.',
    )
    expect(context).toContain('Task: Morning News')
    expect(context).toContain(
      'Original scheduled prompt: Collect the latest important headlines.',
    )
    expect(context).toContain('Status: completed')
    expect(context).toContain('Started: 2026-05-09T14:00:00.000Z')
    expect(context).toContain('Completed: 2026-05-09T14:03:00.000Z')
    expect(context).toContain('Headline one\nHeadline two')
  })

  it('returns null when the run has no result text', () => {
    expect(
      buildScheduledTaskResultChatContext({
        run: { ...baseRun, result: '   ' },
        job: baseJob,
      }),
    ).toBeNull()
  })

  it('falls back when job metadata is missing', () => {
    const context = buildScheduledTaskResultChatContext({
      run: baseRun,
      job: undefined,
    })

    expect(context).toContain('Task: Scheduled Task')
    expect(context).not.toContain('Original scheduled prompt:')
  })
})
