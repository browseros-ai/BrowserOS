import { describe, expect, it } from 'bun:test'
import { taskChangeKey } from './task-change-key'
import type { ExecutionStepRecord, ExecutionTaskRecord } from './types'

function step(
  overrides: Partial<ExecutionStepRecord> = {},
): ExecutionStepRecord {
  return {
    id: 'step-1',
    toolName: 'screenshot',
    order: 0,
    state: 'input-available',
    startedAt: '2026-03-26T10:00:00.000Z',
    previewText: 'Action running',
    ...overrides,
  }
}

function task(
  overrides: Partial<ExecutionTaskRecord> = {},
): ExecutionTaskRecord {
  return {
    id: 'task-1',
    conversationId: 'conv-1',
    promptText: 'do the thing',
    startedAt: '2026-03-26T10:00:00.000Z',
    status: 'running',
    actionCount: 0,
    approvalCount: 0,
    deniedCount: 0,
    errorCount: 0,
    steps: [],
    ...overrides,
  }
}

describe('taskChangeKey', () => {
  it('is stable for tasks that differ only in unread payloads', () => {
    const a = task({ steps: [step({ output: { big: 'A'.repeat(1000) } })] })
    const b = task({ steps: [step({ output: { big: 'B'.repeat(1000) } })] })

    expect(taskChangeKey(a)).toBe(taskChangeKey(b))
  })

  it('changes when a new step is added', () => {
    const before = task({ steps: [step()] })
    const after = task({ steps: [step(), step({ id: 'step-2', order: 1 })] })

    expect(taskChangeKey(before)).not.toBe(taskChangeKey(after))
  })

  it('changes when the last step transitions state', () => {
    const running = task({ steps: [step({ state: 'input-available' })] })
    const done = task({
      steps: [
        step({
          state: 'output-available',
          previewText: 'Completed successfully',
        }),
      ],
    })

    expect(taskChangeKey(running)).not.toBe(taskChangeKey(done))
  })

  it('changes when the task status or preview changes', () => {
    const running = task()
    const completed = task({ status: 'completed', completedAt: 'x' })
    const withPreview = task({ responsePreview: 'partial answer' })

    expect(taskChangeKey(running)).not.toBe(taskChangeKey(completed))
    expect(taskChangeKey(running)).not.toBe(taskChangeKey(withPreview))
  })

  it('distinguishes two tasks in the same state by id', () => {
    const first = task({ id: 'task-1' })
    const second = task({ id: 'task-2' })

    expect(taskChangeKey(first)).not.toBe(taskChangeKey(second))
  })
})
