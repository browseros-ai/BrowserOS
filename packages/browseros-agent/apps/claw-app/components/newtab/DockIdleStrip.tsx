import type { RecentSlice, StatsSlice } from '@/screens/cockpit/newtab.data'
import { RecentPeek } from './RecentPeek'
import { StatGlance } from './StatGlance'

interface DockIdleStripProps {
  stats: StatsSlice
  recent: RecentSlice
}

/** The quiet idle dock: a status line, the compact value stat, and a recent peek. */
export function DockIdleStrip({ stats, recent }: DockIdleStripProps) {
  const tasks = (recent.data?.pages ?? []).flatMap((page) => page.items)

  return (
    <div className="mx-auto w-full max-w-4xl">
      <div className="flex flex-wrap items-baseline justify-between gap-x-4 gap-y-1">
        <span className="text-ink-3 text-sm">No agents running</span>
        <StatGlance isPending={stats.isPending} stats={stats.data} />
      </div>
      <RecentPeek isPending={recent.isPending} tasks={tasks} />
    </div>
  )
}
