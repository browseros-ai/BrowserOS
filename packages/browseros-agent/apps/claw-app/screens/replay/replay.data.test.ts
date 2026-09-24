import { describe, expect, it } from 'bun:test'
import type { ToolDispatchRow } from '@/modules/api/audit.hooks'
import { mapDispatchToFrame, mapTaskStatus } from './replay.data'

const SESSION_START_MS = 1_000_000

function row(overrides: Partial<ToolDispatchRow> = {}): ToolDispatchRow {
  return {
    dispatchId: 1,
    createdAt: SESSION_START_MS + 5_000,
    slug: 'agent',
    label: 'Agent',
    sessionId: 'session-1',
    toolName: 'read',
    ...overrides,
  }
}

describe('mapTaskStatus', () => {
  it('maps cancelled API sessions to the existing stopped run status', () => {
    expect(mapTaskStatus('cancelled')).toBe('stopped')
  })
})

describe('mapDispatchToFrame', () => {
  it.each([
    ['playwright', 'type', 'ran a Playwright script'],
    ['page.goto', 'navigate', 'navigated'],
    ['locator.click', 'click', 'clicked'],
    ['locator.fill', 'type', 'filled'],
    ['locator.press', 'type', 'pressed'],
    ['locator.selectOption', 'type', 'selected an option'],
    ['locator.check', 'click', 'set a checkbox'],
    ['locator.textContent', 'read', 'read text'],
    ['context.newPage', 'read', 'opened a tab'],
    ['context.pages', 'read', 'listed tabs'],
    ['page.waitForLoadState', 'read', 'waited for the page to load'],
    ['page.waitForTimeout', 'read', 'waited'],
    ['page.title', 'read', 'read the page title'],
    ['expect.toBeVisible', 'read', 'checked'],
    ['expect.toHaveText', 'read', 'checked'],
    ['locator.hover', 'read', 'locator.hover'],
    ['page.reload', 'read', 'page.reload'],
    ['run', 'type', 'run'],
    ['act', 'click', 'act'],
  ] as const)(
    'describes %s in the replay without changing the existing icon categories',
    (toolName, verb, caption) => {
      const frame = mapDispatchToFrame(
        row({ toolName }),
        SESSION_START_MS,
        new Map(),
      )
      expect(frame.verb).toBe(verb)
      expect(frame.caption).toBe(caption)
    },
  )

  it('carries the source duration alongside the completion offset', () => {
    const frame = mapDispatchToFrame(
      row({ durationMs: 1_500 }),
      SESSION_START_MS,
      new Map(),
    )

    expect(frame.t).toBe(5)
    expect(frame.durationMs).toBe(1_500)
  })

  it('leaves duration absent when the row never recorded one', () => {
    const frame = mapDispatchToFrame(row(), SESSION_START_MS, new Map())

    expect(frame.durationMs).toBeUndefined()
  })

  it('resolves tab identity through the target map when the row has no tab', () => {
    const frame = mapDispatchToFrame(
      row({ targetId: 'target-a' }),
      SESSION_START_MS,
      new Map([['target-a', 7]]),
    )

    expect(frame.tabId).toBe(7)
  })
})
