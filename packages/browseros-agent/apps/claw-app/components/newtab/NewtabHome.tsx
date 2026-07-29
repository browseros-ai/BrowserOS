import type { CockpitStats } from '@browseros/claw-api'
import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { useNewtabData } from '@/screens/cockpit/newtab.data'
import { AgentGlance } from './AgentGlance'
import { BrandMark } from './BrandMark'
import { OmniSearch } from './OmniSearch'
import { RunningNow } from './RunningNow'
import { TopSites } from './TopSites'

interface NewtabHomeProps {
  sessions: LiveSessionCardRecord[]
  stats?: CockpitStats
  statsPending: boolean
}

/**
 * Adaptive new-tab: a stable brand mark and omnibox over an agent layer that
 * stays quiet when idle (value stat plus a recent peek) and becomes a
 * first-class watch view when sessions are live.
 */
export function NewtabHome({ sessions, stats, statsPending }: NewtabHomeProps) {
  const { topSites, topSitesPending, recent, recentPending } = useNewtabData()
  const isLive = sessions.length > 0

  return (
    <div className="mx-auto flex min-h-[100dvh] w-full max-w-3xl flex-col px-6 pt-[12vh] pb-16 motion-safe:animate-fade-in">
      <div className="flex flex-col items-center gap-6">
        <BrandMark />
        <OmniSearch />
      </div>

      {isLive && (
        <div className="mt-10">
          <RunningNow sessions={sessions} />
        </div>
      )}

      <div className="mt-8">
        <TopSites sites={topSites} isPending={topSitesPending} />
      </div>

      {!isLive && (
        <div className="mt-8">
          <AgentGlance
            stats={stats}
            statsPending={statsPending}
            recent={recent}
            recentPending={recentPending}
          />
        </div>
      )}
    </div>
  )
}
