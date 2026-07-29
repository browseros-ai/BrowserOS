import { CockpitOnboarding } from '@/components/cockpit/CockpitOnboarding'
import { NewtabDock } from '@/components/newtab/NewtabDock'
import { useNewtabData } from './newtab.data'

/** Renders the Claw new-tab: onboarding shells, or the calm dock when ready. */
export function Cockpit() {
  const data = useNewtabData()

  if (data.state !== 'ready') {
    return (
      <div className="mx-auto flex max-w-7xl flex-col px-8 pt-8 pb-16">
        <CockpitOnboarding state={data.state} />
      </div>
    )
  }

  return (
    <NewtabDock
      isLive={data.isLive}
      recent={data.recent}
      sessions={data.sessions}
      stats={data.stats}
      topSites={data.topSites}
    />
  )
}
