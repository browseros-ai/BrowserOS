import { describe, expect, it } from 'bun:test'
import type { TabActivityRecord } from '@/modules/api/tabs.hooks'
import {
  colorForSlug,
  formatRelative,
  formatToolTrail,
  harnessForRow,
  siteOf,
  tabsToActivityRows,
  tabsToAgentActivity,
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

describe('tabsToAgentActivity', () => {
  it('returns a single record when one agent owns one tab', () => {
    const agents = tabsToAgentActivity([record({ targetId: 't1' })])
    expect(agents).toHaveLength(1)
    expect(agents[0].tabs).toHaveLength(1)
    expect(agents[0].currentFocus.targetId).toBe('t1')
    expect(agents[0].toolCount).toBe(1)
    expect(agents[0].lastToolName).toBe('navigate')
  })

  it('rolls three tabs of the same agent into one record', () => {
    const agents = tabsToAgentActivity([
      record({
        targetId: 't1',
        firstToolAt: 1_000_000,
        lastToolAt: 1_000_200,
        lastToolName: 'navigate',
        toolCount: 1,
        recentTools: [{ name: 'navigate', at: 1_000_200 }],
      }),
      record({
        targetId: 't2',
        firstToolAt: 1_000_100,
        lastToolAt: 1_000_500,
        lastToolName: 'snapshot',
        toolCount: 2,
        recentTools: [
          { name: 'navigate', at: 1_000_100 },
          { name: 'snapshot', at: 1_000_500 },
        ],
      }),
      record({
        targetId: 't3',
        firstToolAt: 1_000_050,
        lastToolAt: 1_001_000,
        lastToolName: 'act',
        toolCount: 3,
        recentTools: [
          { name: 'read', at: 1_000_300 },
          { name: 'grep', at: 1_000_600 },
          { name: 'act', at: 1_001_000 },
        ],
      }),
    ])
    expect(agents).toHaveLength(1)
    const agent = agents[0]
    expect(agent.tabs).toHaveLength(3)
    expect(agent.currentFocus.targetId).toBe('t3')
    expect(agent.firstToolAt).toBe(1_000_000)
    expect(agent.lastToolAt).toBe(1_001_000)
    expect(agent.lastToolName).toBe('act')
    expect(agent.toolCount).toBe(6)
    // Merged trail is sorted by `at`, capped at MERGED_TRAIL_CAP=8.
    expect(agent.recentTools.map((t) => t.name)).toEqual([
      'navigate',
      'navigate',
      'read',
      'snapshot',
      'grep',
      'act',
    ])
  })

  it('caps the merged trail at MERGED_TRAIL_CAP=8', () => {
    const events = (offset: number) =>
      Array.from({ length: 5 }, (_, i) => ({
        name: `tool-${offset + i}`,
        at: 1_000_000 + offset * 1000 + i,
      }))
    const agents = tabsToAgentActivity([
      record({ targetId: 't1', recentTools: events(0), toolCount: 5 }),
      record({ targetId: 't2', recentTools: events(5), toolCount: 5 }),
      record({ targetId: 't3', recentTools: events(10), toolCount: 5 }),
    ])
    expect(agents).toHaveLength(1)
    expect(agents[0].recentTools).toHaveLength(8)
    // After the cap, the oldest 7 entries are dropped; the newest 8
    // (sorted by `at`) survive.
    expect(agents[0].recentTools[0].name).toBe('tool-7')
    expect(agents[0].recentTools[7].name).toBe('tool-14')
  })

  it('groups two distinct agent ids into two records sorted by lastToolAt desc', () => {
    const agents = tabsToAgentActivity([
      record({
        targetId: 't1',
        agentId: 'a1',
        slug: 'older',
        lastToolAt: 1_000_000,
      }),
      record({
        targetId: 't2',
        agentId: 'a2',
        slug: 'newer',
        lastToolAt: 2_000_000,
      }),
    ])
    expect(agents).toHaveLength(2)
    expect(agents[0].agentId).toBe('a2')
    expect(agents[1].agentId).toBe('a1')
  })

  it('marks the agent active when at least one of its tabs is active', () => {
    const agents = tabsToAgentActivity([
      record({ targetId: 't1', status: 'idle' }),
      record({ targetId: 't2', status: 'active' }),
    ])
    expect(agents[0].status).toBe('active')
  })

  it('marks the agent idle when all tabs are idle', () => {
    const agents = tabsToAgentActivity([
      record({ targetId: 't1', status: 'idle' }),
      record({ targetId: 't2', status: 'idle' }),
    ])
    expect(agents[0].status).toBe('idle')
  })

  it('falls back to slug when the server-side agentLabel is missing', () => {
    const agents = tabsToAgentActivity([
      record({
        targetId: 't1',
        slug: 'orphan',
        agentLabel: '',
        harness: null,
        color: null,
      }),
    ])
    expect(agents[0].agentLabel).toBe('orphan')
    expect(agents[0].harness).toBe('Claude Code')
    expect(agents[0].color).toBe(colorForSlug('orphan'))
  })

  it('uses the focus tab for the current url/title surface even when older tabs have more activity', () => {
    const agents = tabsToAgentActivity([
      record({
        targetId: 't-busy',
        url: 'https://busy.example.com/',
        title: 'Busy',
        toolCount: 100,
        lastToolAt: 1_000_000,
      }),
      record({
        targetId: 't-fresh',
        url: 'https://fresh.example.com/',
        title: 'Fresh',
        toolCount: 1,
        lastToolAt: 2_000_000,
      }),
    ])
    expect(agents[0].currentFocus.targetId).toBe('t-fresh')
    expect(agents[0].currentFocus.url).toBe('https://fresh.example.com/')
  })

  it('returns an empty array for empty input', () => {
    expect(tabsToAgentActivity([])).toEqual([])
  })
})
