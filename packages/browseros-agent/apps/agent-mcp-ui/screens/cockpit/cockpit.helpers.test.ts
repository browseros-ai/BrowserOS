import { describe, expect, it } from 'bun:test'
import type { TabActivityRecord } from '@/modules/api/tabs.hooks'
import {
  colorForSlug,
  formatRelative,
  siteOf,
  tabsToActivityRows,
  tabsToAgentRows,
} from './cockpit.helpers'

function record(over: Partial<TabActivityRecord> = {}): TabActivityRecord {
  return {
    targetId: 't1',
    pageId: 1,
    url: 'https://example.com/foo',
    title: 'Ex',
    agentId: 'a1',
    slug: 'finance',
    lastToolAt: 1_000_000,
    lastToolName: 'navigate',
    status: 'active',
    ...over,
  }
}

describe('siteOf', () => {
  it('returns the host without leading www', () => {
    expect(siteOf('https://www.example.com/foo')).toBe('example.com')
    expect(siteOf('https://docs.google.com/sheets/abc')).toBe('docs.google.com')
  })

  it('falls back to the raw url for invalid input', () => {
    expect(siteOf('not a url')).toBe('not a url')
  })
})

describe('formatRelative', () => {
  it('returns seconds within a minute', () => {
    expect(formatRelative(99_000, 99_500)).toBe('0s ago')
    expect(formatRelative(95_000, 100_000)).toBe('5s ago')
  })
  it('returns minutes within an hour', () => {
    expect(formatRelative(0, 60_000)).toBe('1m ago')
    expect(formatRelative(0, 3_540_000)).toBe('59m ago')
  })
  it('returns hours within a day', () => {
    expect(formatRelative(0, 3_600_000)).toBe('1h ago')
    expect(formatRelative(0, 23 * 3_600_000)).toBe('23h ago')
  })
  it('returns days otherwise', () => {
    expect(formatRelative(0, 24 * 3_600_000)).toBe('1d ago')
  })
})

describe('colorForSlug', () => {
  it('is deterministic per slug', () => {
    expect(colorForSlug('finance')).toBe(colorForSlug('finance'))
  })
  it('returns a hex string', () => {
    expect(colorForSlug('travel')).toMatch(/^#[0-9A-F]{6}$/i)
  })
})

describe('tabsToAgentRows', () => {
  it('filters out idle records and maps to AgentRow shape', () => {
    const rows = tabsToAgentRows([
      record({ targetId: 't1', status: 'active', slug: 'finance' }),
      record({ targetId: 't2', status: 'idle', slug: 'travel' }),
    ])
    expect(rows.map((r) => r.id)).toEqual(['t1'])
    expect(rows[0]).toMatchObject({
      label: 'finance',
      harness: 'Claude Code',
      site: 'example.com',
      task: 'Ex',
      status: 'running',
    })
  })
})

describe('tabsToActivityRows', () => {
  it('filters out active records and maps to ActivityRow shape', () => {
    const rows = tabsToActivityRows(
      [
        record({ targetId: 't1', status: 'active' }),
        record({
          targetId: 't2',
          status: 'idle',
          slug: 'travel',
          lastToolAt: 950_000,
          lastToolName: 'read',
        }),
      ],
      1_000_000,
    )
    expect(rows.map((r) => r.id)).toEqual(['t2'])
    expect(rows[0]).toMatchObject({
      agentLabel: 'travel',
      status: 'done',
      action: 'read on Ex',
      site: 'example.com',
      when: '50s ago',
    })
  })
})
