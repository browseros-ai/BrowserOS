import { describe, expect, it } from 'bun:test'
import type { TaskStatus, TaskSummary } from '@/modules/api/audit.hooks'
import {
  agentOptionsFor,
  groupByDay,
  needsALook,
  taskWhen,
  tookLabel,
} from './audit.helpers'
import { filtersToParams, paramsToFilters } from './audit.search-params'

function task(
  status: TaskStatus,
  overrides: Partial<TaskSummary> = {},
): TaskSummary {
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
    ...overrides,
  }
}

describe('needsALook', () => {
  it('flags failed and stopped tasks only', () => {
    expect(
      (['live', 'done', 'failed', 'cancelled'] as TaskStatus[]).map((status) =>
        needsALook(task(status)),
      ),
    ).toEqual([false, false, true, true])
  })
})

describe('groupByDay', () => {
  it('splits tasks by local day, newest first, with Today and Yesterday', () => {
    const now = new Date(2026, 9, 23, 0, 30).getTime()
    const groups = groupByDay(
      [
        task('done', {
          sessionId: 'older',
          startedAt: new Date(2026, 9, 20, 9).getTime(),
        }),
        task('done', {
          sessionId: 'late-yesterday',
          startedAt: new Date(2026, 9, 22, 23, 50).getTime(),
        }),
        task('done', {
          sessionId: 'today',
          startedAt: new Date(2026, 9, 23, 0, 5).getTime(),
        }),
        task('done', {
          sessionId: 'early-yesterday',
          startedAt: new Date(2026, 9, 22, 8).getTime(),
        }),
      ],
      now,
    )
    expect(groups.map((group) => group.label).slice(0, 2)).toEqual([
      'Today',
      'Yesterday',
    ])
    expect(groups[2]?.label).toMatch(/October 20/)
    expect(groups[1]?.tasks.map((t) => t.sessionId)).toEqual([
      'late-yesterday',
      'early-yesterday',
    ])
  })
})

describe('agentOptionsFor', () => {
  it('lists each agent once, most active first, and keeps a selected one', () => {
    const options = agentOptionsFor(
      [
        task('done', { slug: 'cursor', label: 'Cursor' }),
        task('done', { slug: 'codex', label: 'codex' }),
        task('done', { slug: 'codex', label: 'Codex' }),
      ],
      'claude-code',
    )
    expect(options.map((option) => option.slug)).toEqual([
      'codex',
      'cursor',
      'claude-code',
    ])
  })
})

describe('History filters in the URL', () => {
  it('round-trips the view, agent and search, and ignores old params', () => {
    const filters = paramsToFilters(
      new URLSearchParams('view=attention&agent=codex&q=flights&status=done'),
    )
    expect(filters).toEqual({
      view: 'attention',
      agentSlug: 'codex',
      search: 'flights',
    })
    expect(filtersToParams(filters).toString()).toBe(
      'agent=codex&view=attention&q=flights',
    )
    expect(paramsToFilters(new URLSearchParams('view=other')).view).toBe('all')
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
