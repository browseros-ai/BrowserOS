import { describe, expect, it } from 'bun:test'
import type { ToolContext } from '../../../src/tools/browser/framework'
import {
  DEFAULT_PAUSE_MS,
  parseWaitMs,
  wait,
} from '../../../src/tools/browser/wait'
import type { ToolResponse } from '../../../src/tools/response'

describe('wait schema', () => {
  it('defaults for to "time" so a bare page just pauses', () => {
    const parsed = wait.input.parse({ page: 2 })
    expect(parsed.for).toBe('time')
    expect(parsed.value).toBeUndefined()
  })

  it('accepts a numeric value (models send numbers, not just strings)', () => {
    const parsed = wait.input.parse({ page: 2, for: 'time', value: 3000 })
    expect(parsed.value).toBe(3000)
  })
})

describe('parseWaitMs', () => {
  it('falls back to the default when value is missing or empty', () => {
    expect(parseWaitMs(undefined, DEFAULT_PAUSE_MS)).toBe(2000)
    expect(parseWaitMs('', DEFAULT_PAUSE_MS)).toBe(2000)
    expect(parseWaitMs('   ', DEFAULT_PAUSE_MS)).toBe(2000)
  })

  it('falls back to the default on garbage instead of erroring', () => {
    // The screenshot bug: a model sent ">" and the tool rejected it.
    expect(parseWaitMs('>', DEFAULT_PAUSE_MS)).toBe(2000)
    expect(parseWaitMs('abc', DEFAULT_PAUSE_MS)).toBe(2000)
    expect(parseWaitMs('-5', DEFAULT_PAUSE_MS)).toBe(2000)
  })

  it('parses a valid millisecond value', () => {
    expect(parseWaitMs('3000', DEFAULT_PAUSE_MS)).toBe(3000)
    expect(parseWaitMs('1500', DEFAULT_PAUSE_MS)).toBe(1500)
    expect(parseWaitMs('0', DEFAULT_PAUSE_MS)).toBe(0)
  })
})

describe('wait handler (for="time")', () => {
  const ctx = { signal: undefined } as unknown as ToolContext
  const response = undefined as unknown as ToolResponse

  it('pauses instead of erroring when value is garbage', async () => {
    const result = await wait.handler(
      { page: 1, for: 'time', value: '>', timeout: 5 },
      ctx,
      response,
    )
    expect(result?.isError).toBeFalsy()
    // Garbage value falls back to the default pause, capped by timeout, and reported.
    expect(result?.structuredContent).toMatchObject({
      matched: true,
      waitedMs: 5,
    })
  })

  it('pauses with no value at all', async () => {
    const result = await wait.handler(
      { page: 1, for: 'time', timeout: 5 },
      ctx,
      response,
    )
    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toMatchObject({ matched: true })
  })
})
