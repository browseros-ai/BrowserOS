import { useRef } from 'react'
import type { ActivityRow } from '@/modules/api/activity.hooks'
import { useTabsActivity } from '@/modules/api/tabs.hooks'
import {
  type AgentActivityRecord,
  tabsToActivityRows,
  tabsToAgentActivity,
} from './cockpit.helpers'

export interface CockpitData {
  agents: AgentActivityRecord[]
  activity: ActivityRow[]
  isPending: boolean
}

/**
 * Single data aggregation hook for the homepage. Per the project
 * convention, the screen calls this and nothing else. PR 3 rolls up
 * the running grid by agent so each card represents one logical run
 * across however many tabs it touches. The recent-activity section
 * stays tab-level by design: "Cowork did read on Stripe 12m ago" is
 * more informative than "Cowork did 14 things". Approvals and
 * handoffs remain on their mocked hooks until later PRs supply them.
 *
 * Sticky focus is applied across polls so the card surface does not
 * flicker when one agent fires a parallel burst of tool calls across
 * several tabs. The hook holds the last poll's per-agent focus in a
 * useRef and passes it back into the rollup as a hint; the rollup
 * keeps that target as focus while it remains in the agent's active
 * set, and re-elects to the freshest tab otherwise.
 */
export function useCockpitData(): CockpitData {
  const tabs = useTabsActivity()

  // The ref is the canonical store for last-seen focus: render N
  // reads from it; render N+1 sees what render N wrote. We mutate in
  // place rather than calling setState so the rollup -> render -> ref
  // loop stays linear and React does not see a second update.
  const stickyFocusRef = useRef<Map<string, string>>(new Map())

  // We pass `Date.now()` at render time; the slight non-determinism
  // is fine for a 1.5s-polling display and avoids dragging a clock
  // injection through the component tree.
  const records = tabs.data?.tabs ?? []
  const now = Date.now()
  const agents = tabsToAgentActivity(
    records.filter((r) => r.status === 'active'),
    { stickyFocus: stickyFocusRef.current },
  )
  // Replace (not mutate) the map so an agent that drops out of the
  // running grid does not pin a stale focus through future polls.
  //
  // Mutating a ref during render looks like a side effect, but it is
  // safe here under both React Strict Mode and concurrent rendering
  // because `tabsToAgentActivity` is pure: for the same `records` and
  // the same `stickyFocus` input it always produces the same focus
  // map. Strict Mode's double-invocation in dev therefore commits the
  // same value, and a concurrent-mode render that is interrupted and
  // retried lands on the same commit too. The alternative of writing
  // through `useEffect` would lag a frame and reintroduce the
  // first-render flicker we are explicitly trying to suppress.
  const nextFocus = new Map<string, string>()
  for (const a of agents) nextFocus.set(a.agentId, a.currentFocus.targetId)
  stickyFocusRef.current = nextFocus

  return {
    agents,
    activity: tabsToActivityRows(records, now),
    isPending: tabs.isPending,
  }
}
