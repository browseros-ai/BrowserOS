import { describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import type { SnapshotDiff } from '@browseros/browser-core/core/snapshot/diff'
import { act } from './act'
import { executeTool } from './framework'

const DIFF: SnapshotDiff = {
  changed: true,
  text: 'DIFF_BODY_MARKER + button "Save" [ref=e1]',
  added: 3,
  removed: 1,
}

// Minimal session: act(kind=focus) only calls input().focus, and the post-action
// diff reads observe().diff() plus pages.getInfo for the origin.
function mockSession(diff: SnapshotDiff): BrowserSession {
  return {
    input: () => ({ focus: async () => {} }),
    observe: () => ({ diff: async () => diff }),
    pages: { getInfo: () => ({ url: 'https://example.com/act' }) },
  } as unknown as BrowserSession
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

describe('act diff control (#2700)', () => {
  it('includes the full diff by default', async () => {
    const result = await executeTool(
      act,
      { page: 0, kind: 'focus', ref: 'e1' },
      { session: mockSession(DIFF) },
    )
    const text = textOf(result)
    expect(text).toContain('ok (focus)')
    expect(text).toContain('[Page 0 diff]')
    expect(text).toContain('DIFF_BODY_MARKER')
  })

  it('skips the diff entirely for diff="none"', async () => {
    const result = await executeTool(
      act,
      { page: 0, kind: 'focus', ref: 'e1', diff: 'none' },
      { session: mockSession(DIFF) },
    )
    const text = textOf(result)
    expect(text).toContain('ok (focus)')
    expect(text).not.toContain('[Page 0 diff]')
    expect(text).not.toContain('Additional context')
    expect(text).not.toContain('DIFF_BODY_MARKER')
  })

  it('returns change counts only for diff="summary"', async () => {
    const result = await executeTool(
      act,
      { page: 0, kind: 'focus', ref: 'e1', diff: 'summary' },
      { session: mockSession(DIFF) },
    )
    const text = textOf(result)
    expect(text).toContain('[Page 0 diff]')
    expect(text).toContain('3 added, 1 removed')
    expect(text).not.toContain('DIFF_BODY_MARKER')
  })
})
