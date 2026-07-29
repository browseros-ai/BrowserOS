import type { LiveSessionCardRecord } from '@/screens/cockpit/cockpit.helpers'
import { useNewtabData } from '@/screens/cockpit/newtab.data'
import { BrandMark } from './BrandMark'
import { IdleGlanceLine } from './IdleGlanceLine'
import { OmniSearch } from './OmniSearch'
import { RunningNow } from './RunningNow'
import { TopSites } from './TopSites'

interface NewtabMonitorProps {
  sessions: LiveSessionCardRecord[]
}

/**
 * State-reordered new-tab column. When agents are live the running block
 * owns the hero of a wider column and search plus top sites demote beneath
 * it; when idle the column narrows to a brand mark, a centered omnibox, top
 * sites, and one quiet status line. The reflow-in of the live block fades and
 * slides through the reduced-motion-gated fade-up utility.
 */
export function NewtabMonitor({ sessions }: NewtabMonitorProps) {
  const data = useNewtabData(sessions)

  if (data.isLive) {
    return (
      <div className="mx-auto flex w-full max-w-5xl flex-col gap-8 pt-6">
        <div className="animate-fade-up">
          <RunningNow
            sessions={data.sessions}
            lead={data.lead}
            rest={data.rest}
            stats={data.stats}
            statsPending={data.statsPending}
          />
        </div>
        <div className="mx-auto w-full max-w-2xl">
          <OmniSearch />
        </div>
        <TopSites topSites={data.topSites} pending={data.topSitesPending} />
      </div>
    )
  }

  return (
    <div className="mx-auto flex w-full max-w-2xl flex-col gap-7 pt-[14vh]">
      <BrandMark />
      <OmniSearch />
      <TopSites topSites={data.topSites} pending={data.topSitesPending} />
      <IdleGlanceLine stats={data.stats} pending={data.statsPending} />
    </div>
  )
}
