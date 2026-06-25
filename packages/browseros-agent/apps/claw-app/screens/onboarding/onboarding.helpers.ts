import {
  CHROME_PROFILES,
  type ChromeProfile,
} from '@/modules/api/onboarding.hooks'

export interface ProfileSelectionSummary {
  selected: readonly ChromeProfile[]
  totalSites: number
  totalLogins: number
}

export function summariseProfileSelection(
  pickedIds: readonly string[],
): ProfileSelectionSummary {
  const selected = CHROME_PROFILES.filter((profile) =>
    pickedIds.includes(profile.id),
  )
  return {
    selected,
    totalSites: selected.reduce((sum, p) => sum + p.sites, 0),
    totalLogins: selected.reduce((sum, p) => sum + p.logins, 0),
  }
}

/** Default picker selection: the two profiles a new user most likely keeps. */
export const DEFAULT_PROFILE_IDS = ['work', 'personal'] as const

export const ONBOARDING_STEP_COUNT = 4

export const CLAUDE_MCP_CLI =
  'claude mcp add --transport http browseros http://127.0.0.1:9000/mcp --scope user'
