import { describe, expect, test } from 'bun:test'
import { type CallOrigin, type ProcessLookup, TaskRule } from './task-rule'

const MINUTE = 60_000

function rule(idleMs = 10 * MINUTE) {
  let n = 0
  return new TaskRule(() => `t${++n}`, idleMs)
}

function process(pid: number): ProcessLookup {
  return { found: true, pid, startedAt: `start-${pid}`, command: 'claude' }
}

const unknown: ProcessLookup = { found: false, reason: 'gone' }

function origin(overrides: Partial<CallOrigin>): CallOrigin {
  return {
    agentTaskId: null,
    client: { name: 'claude-code' },
    process: process(100),
    at: 0,
    ...overrides,
  }
}

describe('TaskRule', () => {
  test('1. a call that names its task joins that task, whatever process sent it', () => {
    const tasks = rule()
    const first = tasks.assign(
      origin({ agentTaskId: 'zuzu-task-7', process: process(1) }),
    )
    const second = tasks.assign(
      origin({ agentTaskId: 'zuzu-task-7', process: process(2) }),
    )
    const other = tasks.assign(
      origin({ agentTaskId: 'zuzu-task-8', process: process(1) }),
    )
    expect(first).toMatchObject({ taskId: 't1', created: true })
    expect(first.identifiedBy).toEqual({
      kind: 'agent-task-id',
      taskId: 'zuzu-task-7',
    })
    expect(second).toMatchObject({ taskId: 't1', created: false })
    expect(other).toMatchObject({ taskId: 't2', created: true })
  })

  test('2. without a task id, the calling process and client name pick the task, for as long as the process lives', () => {
    const tasks = rule()
    const first = tasks.assign(origin({ at: 0 }))
    const later = tasks.assign(origin({ at: 120 * MINUTE }))
    expect(first.identifiedBy).toEqual({
      kind: 'process',
      pid: 100,
      processStartedAt: 'start-100',
      command: 'claude',
    })
    expect(later).toMatchObject({ taskId: first.taskId, created: false })
    // A reused pid is another process: its start time differs.
    const reused = tasks.assign(
      origin({
        process: {
          found: true,
          pid: 100,
          startedAt: 'later',
          command: 'claude',
        },
      }),
    )
    expect(reused.taskId).not.toBe(first.taskId)
  })

  test('3. an unknown process falls back to the client name, with a new task after the idle time', () => {
    const tasks = rule()
    const first = tasks.assign(origin({ process: unknown, at: 0 }))
    tasks.callEnded(first.key, first.taskId, 1_000)
    const soon = tasks.assign(
      origin({ process: unknown, at: 1_000 + 9 * MINUTE }),
    )
    tasks.callEnded(soon.key, soon.taskId, 1_000 + 9 * MINUTE)
    const idle = tasks.assign(
      origin({ process: unknown, at: 1_000 + 20 * MINUTE }),
    )
    expect(first.identifiedBy).toEqual({
      kind: 'client-idle',
      idleMinutes: 10,
      reason: 'gone',
    })
    expect(soon).toMatchObject({ taskId: first.taskId, created: false })
    expect(idle).toMatchObject({ taskId: 't2', created: true })
  })

  test('3. a call still running is not idle time', () => {
    const tasks = rule()
    const long = tasks.assign(origin({ process: unknown, at: 0 }))
    const during = tasks.assign(origin({ process: unknown, at: 15 * MINUTE }))
    expect(during.taskId).toBe(long.taskId)
  })

  test('two agents at once become two tasks and never mix', () => {
    const tasks = rule()
    const seen = new Map<string, string[]>()
    // Two Claude Codes interleaving calls, and two clients the process lookup missed.
    const calls: Array<[string, CallOrigin]> = [
      ['claude A', origin({ process: process(1) })],
      ['claude B', origin({ process: process(2) })],
      ['codex', origin({ process: unknown, client: { name: 'codex' } })],
      ['cursor', origin({ process: unknown, client: { name: 'cursor' } })],
      ['claude B', origin({ process: process(2), at: 5 })],
      ['claude A', origin({ process: process(1), at: 6 })],
      [
        'cursor',
        origin({ process: unknown, client: { name: 'cursor' }, at: 7 }),
      ],
      ['codex', origin({ process: unknown, client: { name: 'codex' }, at: 8 })],
    ]
    for (const [agent, call] of calls) {
      const { taskId } = tasks.assign(call)
      seen.set(agent, [...(seen.get(agent) ?? []), taskId])
    }
    const perAgent = [...seen.values()].map((ids) => new Set(ids))
    expect(perAgent.every((ids) => ids.size === 1)).toBe(true)
    expect(new Set(perAgent.map((ids) => [...ids][0])).size).toBe(4)
  })

  test('a restored task keeps taking its key after a restart', () => {
    const tasks = rule()
    const key = tasks.agentKey({ name: 'claude-code' }, process(100))
    tasks.restore(key, 'from-log', 0)
    expect(tasks.assign(origin({ at: 5 })).taskId).toBe('from-log')
  })
})
