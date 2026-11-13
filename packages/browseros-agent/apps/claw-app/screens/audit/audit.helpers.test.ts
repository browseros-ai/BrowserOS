import { describe, expect, it } from 'bun:test'
import type { TaskStatus, TaskSummary } from '@/modules/api/audit.hooks'
import {
  agentOptionsFor,
  agentsInTasks,
  chipTokens,
  groupByDay,
  pagesLabel,
  taskWhen,
  tookLabel,
  ultrafastTime,
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
  it('lists every agent in the server order and keeps a selected one it lacks', () => {
    const options = agentOptionsFor(
      [
        { slug: 'codex', label: 'Codex', taskCount: 9, lastStartedAt: 2 },
        { slug: 'cursor', label: 'Cursor', taskCount: 1, lastStartedAt: 1 },
      ],
      'claude-code',
    )
    expect(options).toEqual([
      { slug: 'codex', name: 'Codex' },
      { slug: 'cursor', name: 'Cursor' },
      { slug: 'claude-code', name: 'Claude Code' },
    ])
  })

  it('keeps every agent listed while one of them is selected', () => {
    const options = agentOptionsFor(
      [
        { slug: 'codex', label: 'Codex', taskCount: 9, lastStartedAt: 2 },
        { slug: 'cursor', label: 'Cursor', taskCount: 1, lastStartedAt: 1 },
      ],
      'cursor',
    )
    expect(options.map((option) => option.slug)).toEqual(['codex', 'cursor'])
  })
})

describe('agentsInTasks', () => {
  it('counts each agent once, most tasks first, named by its latest task', () => {
    expect(
      agentsInTasks([
        task('done', { slug: 'cursor', label: 'Cursor', startedAt: 5 }),
        task('done', { slug: 'codex', label: 'codex', startedAt: 1 }),
        task('done', { slug: 'codex', label: 'Codex', startedAt: 3 }),
      ]),
    ).toEqual([
      { slug: 'codex', label: 'Codex', taskCount: 2, lastStartedAt: 3 },
      { slug: 'cursor', label: 'Cursor', taskCount: 1, lastStartedAt: 5 },
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

describe('ultrafast run numbers', () => {
  it('says whole seconds under a minute and rounds to minutes after', () => {
    expect(ultrafastTime(15_354)).toBe('15 sec')
    expect(ultrafastTime(8_000)).toBe('8 sec')
    expect(ultrafastTime(200)).toBe('1 sec')
    expect(ultrafastTime(59_400)).toBe('59 sec')
    expect(ultrafastTime(59_600)).toBe('1 min')
    expect(ultrafastTime(125_000)).toBe('2 min')
  })

  it('counts pages, singular for one', () => {
    expect(pagesLabel(1)).toBe('1 page')
    expect(pagesLabel(9)).toBe('9 pages')
  })

  it('keeps chip tokens short', () => {
    expect(chipTokens(640)).toBe('640')
    expect(chipTokens(4_231)).toBe('4.2k')
    expect(chipTokens(2_000)).toBe('2k')
    expect(chipTokens(9_990)).toBe('10k')
    expect(chipTokens(61_942)).toBe('62k')
    expect(chipTokens(180_400)).toBe('180k')
  })
})
