import { useQueryClient } from '@tanstack/react-query'
import { CockpitHero } from '@/components/cockpit/CockpitHero'
import { CockpitOnboarding } from '@/components/cockpit/CockpitOnboarding'
import { RecentActivity } from '@/components/cockpit/RecentActivity'
import { RunningGrid } from '@/components/cockpit/RunningGrid'
import { isUserFacingHarness } from '@/components/harness/harness.types'
import { useTasks } from '@/modules/api/audit.hooks'
import { useBrowserosConnections } from '@/modules/api/connections.hooks'
import { useCockpitData } from './cockpit.data'
import { getOnboardingState } from './cockpit-onboarding.helpers'

const ONBOARDING_PROBE_LIMIT = 1

/** Renders the Claw cockpit homepage. */
export function Cockpit() {
  const { agents } = useCockpitData()
  const queryClient = useQueryClient()

  // Probe the two data sources the onboarding state hinges on. Both
  // queries live in react-query's cache under stable keys, so the
  // downstream components (RecentActivity / Mcp screen) that read
  // the same keys hit the cache instead of refetching.
  const connections = useBrowserosConnections()
  const taskProbe = useTasks({
    variables: { limit: ONBOARDING_PROBE_LIMIT },
  })
  // Only count harnesses that appear on the /mcp screen. Hidden ones
  // (Hermes, OpenClaw, Gemini CLI, retired Claude Desktop) may be
  // preinstalled but are never something the reader intentionally
  // connected, so lighting up 'MCP installed' for them is misleading.
  const hasConnection =
    connections.data?.connections.some(
      (c) => c.installed && isUserFacingHarness(c.harness),
    ) ?? false
  const hasActivity = (taskProbe.data?.pages ?? []).some(
    (p) => p.tasks.length > 0,
  )

  // Wait for both probes to resolve at least once before deciding
  // which shell to render. Otherwise the onboarding block flashes on
  // first paint for returning users whose tasks are still in-flight.
  const probesResolved =
    connections.data !== undefined && taskProbe.data !== undefined
  const state = probesResolved
    ? getOnboardingState({ hasConnection, hasActivity })
    : 'ready'

  if (state !== 'ready') {
    return (
      <div className="mx-auto flex max-w-7xl flex-col px-8 pt-8 pb-16">
        <CockpitOnboarding
          state={state}
          onRefresh={() => {
            void queryClient.invalidateQueries({
              queryKey: useBrowserosConnections.getKey(),
            })
            void queryClient.invalidateQueries({
              queryKey: useTasks.getKey(),
            })
          }}
        />
      </div>
    )
  }

  return (
    <div className="mx-auto flex max-w-7xl flex-col gap-8 px-8 pt-8 pb-16">
      <CockpitHero />
      <RunningGrid agents={agents} />
      <RecentActivity />
    </div>
  )
}
