import { describe, expect, it } from 'bun:test'
import type { TabActivityRecord } from '@/modules/api/tabs.hooks'
import {
  colorForSlug,
  formatRelative,
  formatToolTrail,
  harnessForRow,
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
    firstToolAt: 1_000_000,
    lastToolAt: 1_000_000,
    lastToolName: 'navigate',
    toolCount: 1,
    recentTools: [{ name: 'navigate', at: 1_000_000 }],
    status: 'active',
    agentLabel: 'Finance Ops',
    harness: 'Claude Code',
    color: null,
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

describe('formatToolTrail', () => {
  it('joins tool names with -> and caps to the last N entries', () => {
    const tools = ['navigate', 'snapshot', 'act', 'read', 'grep', 'screenshot']
    expect(
      formatToolTrail(
        tools.map((name, i) => ({ name, at: i })),
        4,
      ),
    ).toBe('act -> read -> grep -> screenshot')
  })
  it('returns an empty string when no recent tools exist', () => {
    expect(formatToolTrail([])).toBe('')
  })
  it('uses the full trail when shorter than the cap', () => {
    expect(formatToolTrail([{ name: 'navigate', at: 0 }], 4)).toBe('navigate')
  })
})

describe('harnessForRow', () => {
  it('passes through known harness names', () => {
    expect(harnessForRow('Cursor')).toBe('Cursor')
    expect(harnessForRow('Codex')).toBe('Codex')
  })
  it('falls back to Claude Code for null', () => {
    expect(harnessForRow(null)).toBe('Claude Code')
  })
  it('falls back to Claude Code for unknown values', () => {
    expect(harnessForRow('Atlas-9000')).toBe('Claude Code')
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
      label: 'Finance Ops',
      harness: 'Claude Code',
      site: 'example.com',
      task: 'Ex',
      status: 'running',
    })
  })

  it('uses the server-supplied agent label and harness', () => {
    const rows = tabsToAgentRows([
      record({
        targetId: 't1',
        status: 'active',
        agentLabel: 'Cowork . File expenses',
        harness: 'Cursor',
      }),
    ])
    expect(rows[0].label).toBe('Cowork . File expenses')
    expect(rows[0].harness).toBe('Cursor')
  })

  it('falls back to slug + Claude Code + hashed colour when the server returned null', () => {
    const rows = tabsToAgentRows([
      record({
        targetId: 't1',
        status: 'active',
        slug: 'orphan',
        agentLabel: '',
        harness: null,
        color: null,
      }),
    ])
    expect(rows[0].label).toBe('orphan')
    expect(rows[0].harness).toBe('Claude Code')
    expect(rows[0].color).toBe(colorForSlug('orphan'))
  })

  it('surfaces the trail, action count, and startedAt on the row', () => {
    const rows = tabsToAgentRows([
      record({
        targetId: 't1',
        status: 'active',
        recentTools: [
          { name: 'navigate', at: 1_000_000 },
          { name: 'snapshot', at: 1_000_100 },
          { name: 'read', at: 1_000_200 },
        ],
        toolCount: 3,
        firstToolAt: 1_000_000,
      }),
    ])
    expect(rows[0].toolCount).toBe(3)
    expect(rows[0].startedAt).toBe(1_000_000)
    expect(rows[0].trail).toBe('navigate -> snapshot -> read')
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
      agentLabel: 'Finance Ops',
      status: 'done',
      action: 'read on Ex',
      site: 'example.com',
      when: '50s ago',
    })
  })

  it('surfaces the trail + action count on idle rows too', () => {
    const rows = tabsToActivityRows(
      [
        record({
          targetId: 't2',
          status: 'idle',
          lastToolAt: 950_000,
          lastToolName: 'read',
          recentTools: [
            { name: 'navigate', at: 900_000 },
            { name: 'snapshot', at: 925_000 },
            { name: 'read', at: 950_000 },
          ],
          toolCount: 3,
        }),
      ],
      1_000_000,
    )
    expect(rows[0].toolCount).toBe(3)
    expect(rows[0].trail).toBe('navigate -> snapshot -> read')
  })
})
