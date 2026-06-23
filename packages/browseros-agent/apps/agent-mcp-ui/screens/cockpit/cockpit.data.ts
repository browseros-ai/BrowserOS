import type { ActivityRow } from '@/modules/api/activity.hooks'
import type { AgentRow } from '@/modules/api/agents.hooks'
import { useTabsActivity } from '@/modules/api/tabs.hooks'
import {
  type ApprovalItem,
  type HandoffItem,
  useApprovals,
  useHandoffs,
} from '@/modules/api/waiting.hooks'
import { tabsToActivityRows, tabsToAgentRows } from './cockpit.helpers'

export interface CockpitData {
  agents: AgentRow[]
  activity: ActivityRow[]
  approvals: ApprovalItem[]
  handoffs: HandoffItem[]
  isPending: boolean
}

/**
 * Single data aggregation hook for the homepage. Per the project
 * convention, the screen calls this and nothing else. PR 1 wires the
 * running grid and recent activity to the real
 * `GET /cockpit/tabs/activity` registry; approvals and handoffs
 * remain on their mocked hooks until later PRs supply them.
 */
export function useCockpitData(): CockpitData {
  const tabs = useTabsActivity()
  const approvals = useApprovals()
  const handoffs = useHandoffs()

  // We pass `Date.now()` at render time; the slight non-determinism
  // is fine for a 1.5s-polling display and avoids dragging a clock
  // injection through the component tree.
  const records = tabs.data?.tabs ?? []
  const now = Date.now()
  return {
    agents: tabsToAgentRows(records),
    activity: tabsToActivityRows(records, now),
    approvals: approvals.data ?? [],
    handoffs: handoffs.data ?? [],
    isPending: tabs.isPending || approvals.isPending || handoffs.isPending,
  }
}
