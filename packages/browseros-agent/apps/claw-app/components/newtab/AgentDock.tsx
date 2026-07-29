import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import type { RecentSlice, StatsSlice } from '@/screens/cockpit/newtab.data'
import { DockIdleStrip } from './DockIdleStrip'
import { DockRunningPanel } from './DockRunningPanel'

interface AgentDockProps {
  sessions: LiveSessionCardRecord[]
  stats: StatsSlice
  recent: RecentSlice
}

/**
 * The low, quiet agent layer. Idle it is a footer-calm strip; live it expands
 * upward into the running panel. Which one shows follows the live-session count.
 */
export function AgentDock({ sessions, stats, recent }: AgentDockProps) {
  return (
    <div className="border-border border-t bg-card-tint/60 px-8 py-4">
      {sessions.length > 0 ? (
        <DockRunningPanel sessions={sessions} stats={stats} />
      ) : (
        <DockIdleStrip recent={recent} stats={stats} />
      )}
    </div>
  )
}
