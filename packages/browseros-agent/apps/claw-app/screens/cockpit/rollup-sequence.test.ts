/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Simulates the homepage's perspective across the lifecycle of a
 * five-tab parallel burst by calling `tabsToAgentActivity` once per
 * synthetic poll and threading the resulting focus map back into the
 * next call. The assertions pin two invariants that PR 3 alone could
 * not guarantee:
 *
 *   - the rolled-up card's `currentFocus` stays anchored to whichever
 *     tab the agent first hit during the burst, instead of flipping
 *     to the freshest tab on every poll;
 *
 *   - the tab-count steps monotonically UP as the parallel landings
 *     arrive (no `5 -> 4 -> 2 -> 0` flicker mid-burst).
 *
 * The test is independent of real time: each "poll" is just a call
 * to `tabsToAgentActivity` with a fresh records array. Active vs.
 * idle is decided by the caller, mirroring how the cockpit's
 * `cockpit.data.ts` filters on `status === 'active'` before rolling
 * up.
 */

import { describe, expect, it } from 'bun:test'
import type { TabActivityRecord } from '@/modules/api/tabs.hooks'
import { tabsToAgentActivity } from './cockpit.helpers'

function tab(over: Partial<TabActivityRecord>): TabActivityRecord {
  return {
    targetId: 't?',
    pageId: 0,
    url: 'https://example.com/',
    title: 'Ex',
    agentId: 'a1',
    slug: 'finance-ops',
    firstToolAt: 0,
    lastToolAt: 0,
    lastToolName: 'snapshot',
    toolCount: 1,
    recentTools: [{ name: 'snapshot', at: 0 }],
    status: 'active',
    agentLabel: 'Finance Ops',
    harness: 'Claude Code',
    color: null,
    screencast: null,
    ...over,
  }
}

function makeBurstTabs(
  targetIds: ReadonlyArray<string>,
  lastToolAtByTarget: ReadonlyMap<string, number>,
): TabActivityRecord[] {
  return targetIds.map((targetId, i) =>
    tab({
      targetId,
      pageId: 7 + i,
      url: `https://${targetId}.example/`,
      title: `Tab ${targetId}`,
      firstToolAt: lastToolAtByTarget.get(targetId) ?? 0,
      lastToolAt: lastToolAtByTarget.get(targetId) ?? 0,
    }),
  )
}

describe('rollup-sequence (sticky focus across polls)', () => {
  it('keeps the card anchored to the first-landed tab through the full burst', () => {
    // Synthetic landings (ms offsets from the burst start) replayed
    // verbatim from the parallel-spawn experiment.
    const landings = new Map<string, number>([
      ['t7', 0],
      ['t8', 2_300],
      ['t9', 4_300],
      ['t10', 6_300],
      ['t11', 6_400],
    ])

    // Eight polls at the homepage's 1.5 s cadence covering the full
    // burst plus a few seconds of "nothing fires" tail.
    const pollSchedule: Array<{
      atMs: number
      visible: ReadonlyArray<string>
    }> = [
      { atMs: 0, visible: [] },
      { atMs: 1_500, visible: ['t7'] },
      { atMs: 3_000, visible: ['t7', 't8'] },
      { atMs: 4_500, visible: ['t7', 't8', 't9'] },
      { atMs: 6_000, visible: ['t7', 't8', 't9'] },
      { atMs: 7_500, visible: ['t7', 't8', 't9', 't10', 't11'] },
      { atMs: 15_000, visible: ['t7', 't8', 't9', 't10', 't11'] },
      { atMs: 25_000, visible: ['t7', 't8', 't9', 't10', 't11'] },
    ]

    let stickyFocus = new Map<string, string>()
    const observedFocus: string[] = []
    const observedCount: number[] = []

    for (const poll of pollSchedule) {
      const visible = poll.visible.map((id) => id)
      const records = makeBurstTabs(visible, landings)
      const agents = tabsToAgentActivity(records, { stickyFocus })
      if (agents.length === 0) {
        observedFocus.push('none')
        observedCount.push(0)
      } else {
        observedFocus.push(agents[0].currentFocus.targetId)
        observedCount.push(agents[0].tabs.length)
      }
      const next = new Map<string, string>()
      for (const a of agents) next.set(a.agentId, a.currentFocus.targetId)
      stickyFocus = next
    }

    // Focus is `t7` (the first-landed) from the moment the agent
    // appears, and never flips during the burst even as fresher tabs
    // land.
    expect(observedFocus).toEqual([
      'none',
      't7',
      't7',
      't7',
      't7',
      't7',
      't7',
      't7',
    ])
    // Tab count rides the high-water mark instead of bouncing.
    expect(observedCount).toEqual([0, 1, 2, 3, 3, 5, 5, 5])
  })

  it('re-elects to the freshest tab when the anchor stops appearing in the active set', () => {
    const landings = new Map<string, number>([
      ['t-anchor', 0],
      ['t-newer', 1_000],
    ])

    const polls: Array<ReadonlyArray<string>> = [
      ['t-anchor'],
      ['t-anchor', 't-newer'],
      ['t-newer'], // t-anchor has aged past the registry's active window
    ]

    let stickyFocus = new Map<string, string>()
    const focuses: string[] = []
    for (const visible of polls) {
      const agents = tabsToAgentActivity(makeBurstTabs(visible, landings), {
        stickyFocus,
      })
      focuses.push(agents[0]?.currentFocus.targetId ?? 'none')
      const next = new Map<string, string>()
      for (const a of agents) next.set(a.agentId, a.currentFocus.targetId)
      stickyFocus = next
    }

    expect(focuses).toEqual(['t-anchor', 't-anchor', 't-newer'])
  })

  it('keeps per-agent focus maps independent across polls', () => {
    const landings = new Map<string, number>([
      ['t1-anchor', 0],
      ['t1-newer', 5_000],
      ['t2-anchor', 0],
      ['t2-newer', 5_000],
    ])

    function buildTabs(
      visible: ReadonlyArray<[string, string]>,
    ): TabActivityRecord[] {
      return visible.map(([agentId, targetId], i) =>
        tab({
          agentId,
          targetId,
          pageId: 100 + i,
          firstToolAt: landings.get(targetId) ?? 0,
          lastToolAt: landings.get(targetId) ?? 0,
        }),
      )
    }

    // Poll 1: each agent has only its anchor.
    let stickyFocus = new Map<string, string>()
    let agents = tabsToAgentActivity(
      buildTabs([
        ['a1', 't1-anchor'],
        ['a2', 't2-anchor'],
      ]),
      { stickyFocus },
    )
    expect(
      Object.fromEntries(
        agents.map((a) => [a.agentId, a.currentFocus.targetId]),
      ),
    ).toEqual({ a1: 't1-anchor', a2: 't2-anchor' })

    const next = new Map<string, string>()
    for (const a of agents) next.set(a.agentId, a.currentFocus.targetId)
    stickyFocus = next

    // Poll 2: each agent gains a fresher tab. Sticky focus holds.
    agents = tabsToAgentActivity(
      buildTabs([
        ['a1', 't1-anchor'],
        ['a1', 't1-newer'],
        ['a2', 't2-anchor'],
        ['a2', 't2-newer'],
      ]),
      { stickyFocus },
    )
    expect(
      Object.fromEntries(
        agents.map((a) => [a.agentId, a.currentFocus.targetId]),
      ),
    ).toEqual({ a1: 't1-anchor', a2: 't2-anchor' })
  })
})
