import { createMutation } from 'react-query-kit'

export interface ChromeProfile {
  id: string
  name: string
  email: string
  /** Number of saved sessions in this profile. */
  sites: number
  /** Subset of `sites` that have password entries (rest are cookie-only). */
  logins: number
}

export const CHROME_PROFILES: readonly ChromeProfile[] = [
  {
    id: 'work',
    name: 'Work',
    email: 'you@your-company.com',
    sites: 31,
    logins: 9,
  },
  {
    id: 'personal',
    name: 'Personal',
    email: 'you+personal@example.com',
    sites: 16,
    logins: 3,
  },
  {
    id: 'testing',
    name: 'Testing',
    email: 'qa@example.com',
    sites: 8,
    logins: 2,
  },
]

export const STARTER_PROMPTS: readonly string[] = [
  'See my May invoices and file expenses on SAP Concur',
  "Pull this week's leads from HubSpot into a summary",
  'Draft and queue 3 LinkedIn posts about the launch',
]

interface ImportVariables {
  /** Profile ids the user ticked in the picker. */
  profileIds: string[]
}

export interface ImportResult {
  importedSites: number
  importedLogins: number
}

/**
 * Mock mutation that simulates importing Chrome sessions. Real
 * implementation will hand off to the BrowserOS Chromium fork's
 * Keychain reader. Body is the only thing that changes when the
 * backend lands; result shape stays the same.
 */
export const useImportChromeSessions = createMutation<
  ImportResult,
  ImportVariables
>({
  mutationFn: async ({ profileIds }) => {
    const selected = CHROME_PROFILES.filter((profile) =>
      profileIds.includes(profile.id),
    )
    const importedSites = selected.reduce((sum, p) => sum + p.sites, 0)
    const importedLogins = selected.reduce((sum, p) => sum + p.logins, 0)
    await new Promise((resolve) => setTimeout(resolve, 1500))
    return { importedSites, importedLogins }
  },
})

export interface ConnectResult {
  /** Number of tools the harness sees once BrowserOS is registered as a connector. */
  toolCount: number
  /** Scope the connector was registered with (eventually `user` or `workspace`). */
  scope: 'user' | 'workspace'
}

/**
 * Mock mutation for the "Add to Claude" handoff. Real surface will
 * invoke Claude Desktop's connector-install API or fall back to the
 * `claude mcp add` CLI command surfaced in the wizard's right rail.
 */
export const useConnectToClaude = createMutation<ConnectResult>({
  mutationFn: async () => {
    await new Promise((resolve) => setTimeout(resolve, 1700))
    return { toolCount: 68, scope: 'user' }
  },
})
