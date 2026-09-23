import { describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { executeTool } from './framework'
import { resolveTimeWaitMs, wait } from './wait'

// A for="time" pause touches no session; only executeTool's tail reads pages.
const timeCtx = {
  session: { pages: {} } as unknown as BrowserSession,
}

function textOf(result: { content?: unknown } | undefined): string {
  if (!Array.isArray(result?.content)) return ''
  return result.content
    .filter(
      (item): item is { type: 'text'; text: string } =>
        typeof item === 'object' &&
        item !== null &&
        'type' in item &&
        (item as { type: unknown }).type === 'text' &&
        'text' in item &&
        typeof (item as { text: unknown }).text === 'string',
    )
    .map((item) => item.text)
    .join('\n')
}

describe('resolveTimeWaitMs (#2701)', () => {
  it('honors a long-but-valid value the old 30s clamp would have dropped', () => {
    // 75000ms is the reporter's case; it must be honored, not silently clamped to 30000.
    expect(resolveTimeWaitMs('75000')).toEqual({ waitMs: 75000 })
  })

  it('rejects a value above the cap and names the cap', () => {
    const resolved = resolveTimeWaitMs('120001')
    expect(resolved).toHaveProperty('error')
    if ('error' in resolved) {
      expect(resolved.error).toContain('120000')
      expect(resolved.error).toContain('120001')
    }
  })

  it('defaults a missing value to the default pause', () => {
    expect(resolveTimeWaitMs(undefined)).toEqual({ waitMs: 2000 })
  })
})

describe('wait for="time"', () => {
  it('waits and reports the elapsed ms for a small value', async () => {
    const result = await executeTool(
      wait,
      { page: 0, for: 'time', value: 5 },
      timeCtx,
    )
    expect(result.isError).toBeFalsy()
    expect(result.structuredContent).toMatchObject({
      matched: true,
      waitedMs: 5,
    })
    expect(textOf(result)).toContain('waited 5ms')
  })

  it('rejects an over-cap value instead of silently clamping', async () => {
    const result = await executeTool(
      wait,
      { page: 0, for: 'time', value: 120001 },
      timeCtx,
    )
    expect(result.isError).toBe(true)
    expect(textOf(result)).toContain('120000')
  })
})
