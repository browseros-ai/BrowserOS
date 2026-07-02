import { createMutation, createQuery } from 'react-query-kit'
import type { RunStatus } from '@/lib/status'
import type {
  Harness,
  NewAgentValues,
} from '@/screens/new-agent/new-agent.schemas'
import { api } from './client'
import { parseResponse } from './parseResponse'

/**
 * Cockpit's running grid still reads mock data; this hook becomes
 * Phase 4's projection over the runs store. Leave the shape and
 * fixtures untouched until then.
 */
export interface AgentRow {
  id: string
  /** Display label, e.g. "Cowork . File expenses". */
  label: string
  harness: Harness
  site: string
  task: string
  status: RunStatus
  liveLine: string
  /** Hex color used for the per-agent dot in cross-agent activity rows. */
  color: string
  /**
   * Total tool dispatches recorded against this tab since the agent
   * first touched it. Surfaced as a small badge on the running card.
   */
  toolCount?: number
  /** Epoch ms of the first tool dispatch on this tab. Surfaces as "started Xm ago". */
  startedAt?: number
  /**
   * Short formatted trail of the last few tool names, e.g.
   * `navigate -> snapshot -> act`. Empty string when no trail is
   * available; the card hides the row in that case.
   */
  trail?: string
}

const MOCK_AGENTS: AgentRow[] = [
  {
    id: 'cld-concur',
    label: 'Cowork . File expenses',
    harness: 'Claude Code',
    site: 'concur.com',
    task: 'See my May invoices and file expenses on SAP Concur',
    status: 'needs-ok',
    liveLine: 'Filling 4 expense lines',
    color: '#0f3e17',
  },
  {
    id: 'cld-li',
    label: 'Cowork . LinkedIn posts',
    harness: 'Claude Code',
    site: 'linkedin.com',
    task: 'Draft and queue 3 LinkedIn posts about the launch',
    status: 'running',
    liveLine: 'Typing the 2nd post in the composer',
    color: '#2F6FE0',
  },
  {
    id: 'cdx-sheet',
    label: 'Codex . Pricing research',
    harness: 'Codex',
    site: 'docs.google.com',
    task: 'Compile competitor pricing into a Google Sheet',
    status: 'running',
    liveLine: 'Pasting row 9 of 12 into the sheet',
    color: '#1F8A4C',
  },
]

export const useAgents = createQuery<AgentRow[]>({
  queryKey: ['agents'],
  fetcher: () =>
    new Promise((resolve) => setTimeout(() => resolve(MOCK_AGENTS), 60)),
})

/**
 * Result of the harness-install side-effect that runs on `POST /agents`.
 * `installed: false` means the profile was saved but the harness's
 * MCP config could not be written (locked file, BrowserOS-internal
 * harness, agent-mcp-manager error). The wizard's success card uses
 * `message` directly so backend wording flows through.
 */
export interface HarnessInstallOutcome {
  installed: boolean
  message: string
  configPath?: string
}

export interface CreatedAgent {
  id: string
  name: string
  harness: NewAgentValues['harness']
  slug: string
  mcpUrl: string
  cliCommand: string
  harnessInstall: HarnessInstallOutcome
}

export const useCreateAgent = createMutation<CreatedAgent, NewAgentValues>({
  mutationFn: async (values) => {
    const response = await api.agents.$post({ json: values })
    return parseResponse<CreatedAgent>(response)
  },
})

export type AgentProfileStatus = 'configured' | 'paused' | 'disabled'

export interface AgentProfile {
  id: string
  /** Connector label, e.g. "Cowork . Finance ops". */
  name: string
  harness: AgentRow['harness']
  /** "All sites from current profile" / "All my logins" / "Selective". */
  loginScopeLabel: string
  loginCount: number
  /** Number of selected ACL rules (seed + custom). */
  aclRuleCount: number
  /** Number of approval categories set to Block. */
  blockedActionCount: number
  /** Number of "Always allow" grants accumulated. */
  alwaysAllowCount: number
  /** Relative time, e.g. "2m ago", "Yesterday 17:42", "Never run". */
  lastRunAt: string
  status: AgentProfileStatus
  /** MCP endpoint URL handed to the harness. */
  mcpUrl: string
}

export const useAgentProfiles = createQuery<AgentProfile[]>({
  queryKey: ['agent-profiles'],
  fetcher: async () => {
    const response = await api.agents.$get()
    return parseResponse<AgentProfile[]>(response)
  },
})

interface DeleteAgentVariables {
  id: string
}

export const useDeleteAgent = createMutation<
  DeleteAgentVariables,
  DeleteAgentVariables
>({
  mutationFn: async ({ id }) => {
    const response = await api.agents[':id'].$delete({ param: { id } })
    return parseResponse<DeleteAgentVariables>(response)
  },
})

interface RegenerateMcpVariables {
  id: string
}

interface RegenerateMcpResult {
  id: string
  mcpUrl: string
}

export const useRegenerateMcpUrl = createMutation<
  RegenerateMcpResult,
  RegenerateMcpVariables
>({
  mutationFn: async ({ id }) => {
    const response = await api.agents[':id']['mcp-url:regenerate'].$post({
      param: { id },
    })
    return parseResponse<RegenerateMcpResult>(response)
  },
})

interface UseAgentProfileDetailVariables {
  id: string
}

export const useAgentProfileDetail = createQuery<
  NewAgentValues | null,
  UseAgentProfileDetailVariables
>({
  queryKey: ['agent-profile-detail'],
  fetcher: async ({ id }) => {
    const response = await api.agents[':id'].$get({ param: { id } })
    if (response.status === 404) return null
    return parseResponse<NewAgentValues>(response)
  },
})

interface UpdateAgentVariables extends NewAgentValues {
  id: string
}

export const useUpdateAgent = createMutation<
  UpdateAgentVariables,
  UpdateAgentVariables
>({
  mutationFn: async ({ id, ...values }) => {
    const response = await api.agents[':id'].$patch({
      param: { id },
      json: values,
    })
    return parseResponse<UpdateAgentVariables>(response)
  },
})
