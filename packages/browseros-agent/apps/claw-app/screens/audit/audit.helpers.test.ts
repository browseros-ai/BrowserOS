import { describe, expect, it } from 'bun:test'
import type { TaskStatus, TaskSummary } from '@/modules/api/audit.hooks'
import { statusOptions } from './audit.helpers'
import { paramsToFilters } from './audit.search-params'

function task(status: TaskStatus): TaskSummary {
  return {
    sessionId: `${status}-session`,
    slug: 'codex',
    label: 'Codex',
    name: 'Task',
    startedAt: 1,
    durationMs: 1,
    dispatchCount: 1,
    toolSequence: ['tabs'],
    status,
    errorCount: 0,
  }
}

describe('statusOptions', () => {
  it('counts cancelled sessions alongside the existing statuses', () => {
    expect(
      statusOptions([
        task('live'),
        task('done'),
        task('failed'),
        task('cancelled'),
        task('cancelled'),
      ]),
    ).toEqual([
      { status: 'live', count: 1 },
      { status: 'done', count: 1 },
      { status: 'failed', count: 1 },
      { status: 'cancelled', count: 2 },
    ])
  })
})

describe('paramsToFilters', () => {
  it('preserves the cancelled status filter', () => {
    expect(
      paramsToFilters(new URLSearchParams('status=cancelled')).status,
    ).toBe('cancelled')
  })
})
