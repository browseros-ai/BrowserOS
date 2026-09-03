import { describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { executeTool } from './framework'
import { history } from './history'

const entries = [
  {
    id: 'entry-1',
    url: 'https://example.test/first',
    title: 'First visit',
    lastVisitTime: 1_785_456_000_000,
    visitCount: 4,
    typedCount: 1,
  },
  {
    id: 'entry-2',
    url: 'https://example.test/second',
    title: '',
    lastVisitTime: 1_785_369_600_000,
    visitCount: 1,
    typedCount: 0,
  },
]

function fakeSession(resultEntries: typeof entries) {
  const calls: Array<{ method: string; params: unknown }> = []
  const session = {
    cdp: async (method: string, params: unknown) => {
      calls.push({ method, params })
      return { entries: resultEntries }
    },
  } as unknown as BrowserSession
  return { session, calls }
}

function textOf(result: Awaited<ReturnType<typeof executeTool>>) {
  return result.content
    .filter(
      (item): item is { type: 'text'; text: string } =>
        item.type === 'text' && typeof item.text === 'string',
    )
    .map((item) => item.text)
    .join('\n')
}

describe('history tool', () => {
  it('forwards maxResults and returns full entries', async () => {
    const { session, calls } = fakeSession(entries)

    const result = await executeTool(history, { maxResults: 2 }, { session })

    expect(result.isError).toBeFalsy()
    expect(result.structuredContent).toEqual({ entries, count: 2 })
    // History titles/URLs are site-derived; the output is fenced as untrusted.
    expect(textOf(result)).toContain('UNTRUSTED_PAGE_CONTENT')
    expect(textOf(result)).toContain('First visit')
    expect(textOf(result)).toContain('https://example.test/first')
    expect(textOf(result)).toContain('last visited 2026-07-31T00:00:00Z')
    expect(textOf(result)).toContain('4 visits')
    expect(textOf(result)).toContain('https://example.test/second')
    expect(calls).toEqual([
      { method: 'History.getRecent', params: { maxResults: 2 } },
    ])
  })

  it('defaults maxResults to 100 and handles empty history', async () => {
    const { session, calls } = fakeSession([])

    const result = await executeTool(history, {}, { session })

    expect(result.isError).toBeFalsy()
    expect(textOf(result)).toBe('(no history)')
    expect(result.structuredContent).toEqual({ entries: [], count: 0 })
    expect(calls).toEqual([
      { method: 'History.getRecent', params: { maxResults: 100 } },
    ])
  })

  it('rejects invalid and unknown inputs', async () => {
    const { session, calls } = fakeSession([])

    for (const args of [
      { maxResults: 0 },
      { maxResults: -1 },
      { maxResults: 1.5 },
      { maxResults: '10' },
      { query: 'example' },
    ]) {
      const result = await executeTool(history, args, { session })
      expect(result.isError).toBe(true)
      expect(textOf(result)).toStartWith('Invalid arguments for history:')
    }
    expect(calls).toEqual([])
  })
})
