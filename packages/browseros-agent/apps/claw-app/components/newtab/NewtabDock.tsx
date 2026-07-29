import type { NewtabData } from '@/screens/cockpit/newtab.data'
import { AgentDock } from './AgentDock'
import { BrandMark } from './BrandMark'
import { OmniSearch } from './OmniSearch'
import { TopSites } from './TopSites'

type NewtabDockProps = Omit<NewtabData, 'state'>

/**
 * The full new-tab: an anchored search focal at the top, a calm spacer, and the
 * agent dock at the bottom. Only the spacer flexes, so the focal's position is
 * invariant to the dock's height. Idle to live changes the dock, the spacer
 * absorbs the delta, and the focal does not move.
 */
export function NewtabDock({
  sessions,
  isLive,
  stats,
  recent,
  topSites,
}: NewtabDockProps) {
  return (
    <div className="flex min-h-[100dvh] animate-fade-in flex-col">
      <NewtabFocal
        isLive={isLive}
        runningCount={sessions.length}
        topSites={topSites}
      />
      <div aria-hidden className="flex-1" />
      <AgentDock recent={recent} sessions={sessions} stats={stats} />
    </div>
  )
}

interface NewtabFocalProps {
  isLive: boolean
  runningCount: number
  topSites: NewtabData['topSites']
}

function NewtabFocal({ isLive, runningCount, topSites }: NewtabFocalProps) {
  const statusLine = isLive ? liveStatusLine(runningCount) : undefined

  return (
    <div className="flex flex-col items-center gap-6 px-8 pt-24">
      <BrandMark statusLine={statusLine} />
      <OmniSearch />
      <TopSites isPending={topSites.isPending} sites={topSites.data ?? []} />
    </div>
  )
}

function liveStatusLine(count: number): string {
  return count === 1 ? '1 agent working now' : `${count} agents working now`
}
