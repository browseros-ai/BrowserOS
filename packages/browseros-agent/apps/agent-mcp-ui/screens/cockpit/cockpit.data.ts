import type { ActivityRow } from '@/modules/api/activity.hooks'
import { useTabsActivity } from '@/modules/api/tabs.hooks'
import {
  type ApprovalItem,
  type HandoffItem,
  useApprovals,
  useHandoffs,
} from '@/modules/api/waiting.hooks'
import {
  type AgentActivityRecord,
  tabsToActivityRows,
  tabsToAgentActivity,
} from './cockpit.helpers'

export interface CockpitData {
  agents: AgentActivityRecord[]
  activity: ActivityRow[]
  approvals: ApprovalItem[]
  handoffs: HandoffItem[]
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
    // The rollup only considers active records so an agent that is
    // currently idle drops out of the running grid; its individual
    // tabs still appear in the recent activity list below.
    agents: tabsToAgentActivity(records.filter((r) => r.status === 'active')),
    activity: tabsToActivityRows(records, now),
    approvals: approvals.data ?? [],
    handoffs: handoffs.data ?? [],
    isPending: tabs.isPending || approvals.isPending || handoffs.isPending,
  }
}
