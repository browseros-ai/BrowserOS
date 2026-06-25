import type { HarnessAdapterHealth } from './agent-harness-types'

export type AdapterHealthTone = 'ready' | 'warning' | 'danger'

export function adapterHealthLabel(health: HarnessAdapterHealth): string {
  switch (health.readiness) {
    case 'ready':
      return 'Ready'
    case 'needs-auth':
      return 'Login needed'
    case 'needs-install':
      return 'Install needed'
    case 'will-fetch-package':
      return 'Fetch on first run'
    case 'diagnostic-warning':
      return 'Check warning'
    case 'unknown':
      return 'Check failed'
    default:
      return health.healthy ? 'Ready' : 'Unavailable'
  }
}

export function adapterHealthTone(
  health: HarnessAdapterHealth,
): AdapterHealthTone {
  switch (health.readiness) {
    case 'ready':
      return 'ready'
    case 'needs-install':
    case 'needs-auth':
      return 'danger'
    case 'diagnostic-warning':
      return 'warning'
    default:
      return health.healthy ? 'ready' : 'warning'
  }
}

export function adapterHealthMeta(health: HarnessAdapterHealth): string | null {
  const parts: string[] = []
  if (health.version) parts.push(health.version)
  const launch = launchSourceLabel(health.adapterLaunchSource)
  if (launch) parts.push(launch)
  return parts.length > 0 ? parts.join(' · ') : null
}

function launchSourceLabel(
  source: HarnessAdapterHealth['adapterLaunchSource'],
): string | null {
  switch (source) {
    case 'bundled-bun':
      return 'bundled Bun'
    case 'host-npx':
      return 'npx'
    case 'host-cli':
      return 'host CLI'
    default:
      return null
  }
}
