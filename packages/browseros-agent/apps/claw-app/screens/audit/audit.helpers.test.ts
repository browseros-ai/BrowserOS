import { describe, expect, it } from 'bun:test'
import type { TaskStatus, TaskSummary } from '@/modules/api/audit.hooks'
import { statusOptions, taskWhen, tookLabel } from './audit.helpers'
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

describe('taskWhen', () => {
  it('shows the time today, Yesterday across midnight, then the date', () => {
    const now = new Date(2026, 9, 23, 0, 30).getTime()
    expect(taskWhen(new Date(2026, 9, 23, 0, 5).getTime(), now)).toMatch(
      /12:05/,
    )
    expect(taskWhen(new Date(2026, 9, 22, 23, 50).getTime(), now)).toBe(
      'Yesterday',
    )
    expect(taskWhen(new Date(2026, 9, 20, 9, 0).getTime(), now)).not.toBe(
      'Yesterday',
    )
  })
})

describe('tookLabel', () => {
  it('rounds to minutes and hours', () => {
    expect(tookLabel(20_000)).toBe('took under a minute')
    expect(tookLabel(4 * 60_000)).toBe('took 4 min')
    expect(tookLabel(125 * 60_000)).toBe('took 2 h 5 min')
  })
})
