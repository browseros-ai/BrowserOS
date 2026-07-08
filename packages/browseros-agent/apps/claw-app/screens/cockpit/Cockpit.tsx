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

  // Probe the two data sources the onboarding state hinges on. Both
  // queries live in react-query's cache under stable keys, so the
  // downstream components (RecentActivity / Mcp screen) that read
  // the same keys hit the cache instead of refetching.
  const connections = useBrowserosConnections()
  const taskProbe = useTasks({
    variables: { limit: ONBOARDING_PROBE_LIMIT },
    // Scoped to the cockpit probe: while the reader sits on the
    // first-run / waiting shells the poll needs to feel live so the
    // 'ready' handoff lands within a few seconds of their first
    // agent write. React-query's default polling elsewhere in the
    // app is unchanged.
    refetchInterval: 4000,
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
  // DO NOT COMMIT — local override so the first-run onboarding
  // renders regardless of connection/activity state. Revert this
  // block back to the probesResolved ternary before pushing.
  // Append `#/onboarding=waiting` to the URL to preview the waiting
  // variant instead; anything else lands on first-run.
  void probesResolved
  void getOnboardingState
  void hasConnection
  void hasActivity
  const state = (
    typeof window !== 'undefined' &&
    window.location.hash.includes('onboarding=waiting')
      ? 'waiting'
      : 'first-run'
  ) as 'first-run' | 'waiting' | 'ready'

  if (state !== 'ready') {
    return (
      <div className="mx-auto flex max-w-7xl flex-col px-8 pt-8 pb-16">
        <CockpitOnboarding state={state} />
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
