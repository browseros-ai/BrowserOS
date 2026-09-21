/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it } from 'bun:test'
import type { ErrorEvent } from '@sentry/bun'
import { createEventBudget, throttleKey } from '../../src/lib/sentry-throttle'

function errorEvent(overrides: Partial<ErrorEvent> = {}): ErrorEvent {
  return {
    exception: {
      values: [
        {
          type: 'Error',
          value: 'boom',
          stacktrace: { frames: [{ function: 'doThing' }] },
        },
      ],
    },
    ...overrides,
  }
}

describe('throttleKey', () => {
  it('uses an explicit fingerprint when present', () => {
    expect(
      throttleKey(errorEvent({ fingerprint: ['tool-execution', 'navigate'] })),
    ).toBe('tool-execution|navigate')
  })

  it('falls back to exception type and innermost frame', () => {
    expect(throttleKey(errorEvent())).toBe('Error|doThing')
  })

  it('falls back to the transaction when there is no stack', () => {
    expect(
      throttleKey(
        errorEvent({
          exception: { values: [{ type: 'SQLiteError', value: 'no table' }] },
          transaction: 'GET /agents',
        }),
      ),
    ).toBe('SQLiteError|GET /agents')
  })

  it('is stable for two events from the same failure', () => {
    expect(throttleKey(errorEvent())).toBe(throttleKey(errorEvent()))
  })
})

describe('createEventBudget', () => {
  it('admits up to maxPerKey events then drops the rest in the window', () => {
    const budget = createEventBudget({
      windowMs: 1_000,
      maxPerKey: 2,
      maxKeys: 100,
    })
    const event = errorEvent()
    expect(budget.admit(event, 0)).toBe(0)
    expect(budget.admit(event, 10)).toBe(0)
    expect(budget.admit(event, 20)).toBeNull()
    expect(budget.admit(event, 30)).toBeNull()
  })

  it('gives each issue its own independent budget', () => {
    const budget = createEventBudget({
      windowMs: 1_000,
      maxPerKey: 1,
      maxKeys: 100,
    })
    const enospc = errorEvent({
      exception: { values: [{ type: 'Error', value: 'ENOSPC' }] },
      transaction: 'flush',
    })
    const noTable = errorEvent({
      exception: { values: [{ type: 'SQLiteError', value: 'no table' }] },
      transaction: 'GET /agents',
    })
    expect(budget.admit(enospc, 0)).toBe(0)
    expect(budget.admit(enospc, 5)).toBeNull()
    // A different issue is unaffected by the first one exhausting its budget.
    expect(budget.admit(noTable, 6)).toBe(0)
  })

  it('admits again after the window and reports how many were suppressed', () => {
    const budget = createEventBudget({
      windowMs: 1_000,
      maxPerKey: 1,
      maxKeys: 100,
    })
    const event = errorEvent()
    expect(budget.admit(event, 0)).toBe(0)
    // Three more arrive while over budget, all dropped.
    expect(budget.admit(event, 100)).toBeNull()
    expect(budget.admit(event, 200)).toBeNull()
    expect(budget.admit(event, 300)).toBeNull()
    // Next window: the representative event carries the suppressed count.
    expect(budget.admit(event, 1_000)).toBe(3)
    // And the count is cleared once reported.
    expect(budget.admit(event, 1_100)).toBeNull()
    expect(budget.admit(event, 2_000)).toBe(1)
  })

  it('stays bounded and re-admits an evicted issue as fresh', () => {
    const budget = createEventBudget({
      windowMs: 10_000,
      maxPerKey: 1,
      maxKeys: 2,
    })
    const key = (n: number) =>
      errorEvent({
        exception: { values: [{ type: 'Error', value: `issue-${n}` }] },
        transaction: `t-${n}`,
      })
    // Exhaust the oldest issue, then push past maxKeys with newer distinct ones.
    expect(budget.admit(key(0), 0)).toBe(0)
    expect(budget.admit(key(0), 1)).toBeNull()
    budget.admit(key(1), 2)
    budget.admit(key(2), 3)
    // issue-0 was evicted to honor the bound, so it admits fresh again.
    expect(budget.admit(key(0), 4)).toBe(0)
  })
})
