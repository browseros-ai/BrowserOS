import { nanoid } from 'nanoid'
import { createMutation, createQuery } from 'react-query-kit'
import type { RunStatus } from '@/lib/status'
import {
  buildCliCommand,
  buildMcpUrl,
  IMPORTED_SITES,
  SEED_ACL_RULES,
  toSlug,
} from '@/screens/new-agent/new-agent.helpers'
import {
  APPROVAL_CATEGORIES,
  type ApprovalVerdict,
  type LoginMode,
  type NewAgentValues,
} from '@/screens/new-agent/new-agent.schemas'

export interface AgentRow {
  id: string
  /** Display label, e.g. "Cowork . File expenses". */
  label: string
  harness: 'Claude Cowork' | 'Codex' | 'Hermes' | 'OpenClaw' | 'Gemini CLI'
  site: string
  task: string
  status: RunStatus
  liveLine: string
  /** Hex color used for the per-agent dot in cross-agent activity rows. */
  color: string
}

/**
 * Mock data. Shape matches what the eventual
 * `agent-mcp-interface` /agents endpoint will return. Replacing
 * `fetcher` with a real `$get`-then-parseResponse call is the only
 * change this hook needs when the backend route lands.
 */
const MOCK_AGENTS: AgentRow[] = [
  {
    id: 'cld-concur',
    label: 'Cowork . File expenses',
    harness: 'Claude Cowork',
    site: 'concur.com',
    task: 'See my May invoices and file expenses on SAP Concur',
    status: 'needs-ok',
    liveLine: 'Filling 4 expense lines',
    color: '#F26B2A',
  },
  {
    id: 'cld-li',
    label: 'Cowork . LinkedIn posts',
    harness: 'Claude Cowork',
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

export interface CreatedAgent {
  id: string
  name: string
  harness: NewAgentValues['harness']
  slug: string
  mcpUrl: string
  cliCommand: string
}

/**
 * Mock createAgent mutation. Mirrors the eventual hono-rpc surface so
 * swapping `mutationFn` for a real `$post`-then-parseResponse call is
 * a body-only change. The simulated latency keeps the optimistic UI
 * states honest.
 */
export const useCreateAgent = createMutation<CreatedAgent, NewAgentValues>({
  mutationFn: async (values) => {
    await new Promise((resolve) => setTimeout(resolve, 600))
    const slug = toSlug(values.name || values.harness)
    return {
      id: nanoid(8),
      name: values.name,
      harness: values.harness,
      slug,
      mcpUrl: buildMcpUrl(slug),
      cliCommand: buildCliCommand(slug),
    }
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

const MOCK_AGENT_PROFILES: AgentProfile[] = [
  {
    id: 'cld-concur',
    name: 'Cowork . Finance ops',
    harness: 'Claude Cowork',
    loginScopeLabel: 'Current profile (47)',
    loginCount: 47,
    aclRuleCount: 3,
    blockedActionCount: 1,
    alwaysAllowCount: 4,
    lastRunAt: '2m ago',
    status: 'configured',
    mcpUrl: 'http://127.0.0.1:9000/mcp/cowork-finance-ops',
  },
  {
    id: 'cld-li',
    name: 'Cowork . Social posts',
    harness: 'Claude Cowork',
    loginScopeLabel: 'Selective (5)',
    loginCount: 5,
    aclRuleCount: 5,
    blockedActionCount: 2,
    alwaysAllowCount: 1,
    lastRunAt: '4m ago',
    status: 'configured',
    mcpUrl: 'http://127.0.0.1:9000/mcp/cowork-social-posts',
  },
  {
    id: 'cdx-sheet',
    name: 'Codex . Pricing research',
    harness: 'Codex',
    loginScopeLabel: 'Selective (8)',
    loginCount: 8,
    aclRuleCount: 4,
    blockedActionCount: 1,
    alwaysAllowCount: 2,
    lastRunAt: '8m ago',
    status: 'configured',
    mcpUrl: 'http://127.0.0.1:9000/mcp/codex-pricing-research',
  },
  {
    id: 'cdx-leads',
    name: 'Codex . Pipeline digest',
    harness: 'Codex',
    loginScopeLabel: 'Current profile (47)',
    loginCount: 47,
    aclRuleCount: 3,
    blockedActionCount: 1,
    alwaysAllowCount: 3,
    lastRunAt: '34m ago',
    status: 'configured',
    mcpUrl: 'http://127.0.0.1:9000/mcp/codex-pipeline-digest',
  },
  {
    id: 'hrm-stripe',
    name: 'Hermes . Refunds',
    harness: 'Hermes',
    loginScopeLabel: 'Selective (2)',
    loginCount: 2,
    aclRuleCount: 5,
    blockedActionCount: 2,
    alwaysAllowCount: 0,
    lastRunAt: '1h ago',
    status: 'paused',
    mcpUrl: 'http://127.0.0.1:9000/mcp/hermes-refunds',
  },
  {
    id: 'hrm-notion',
    name: 'Hermes . Weekly recap',
    harness: 'Hermes',
    loginScopeLabel: 'Current profile (47)',
    loginCount: 47,
    aclRuleCount: 2,
    blockedActionCount: 1,
    alwaysAllowCount: 1,
    lastRunAt: 'Yesterday 09:11',
    status: 'configured',
    mcpUrl: 'http://127.0.0.1:9000/mcp/hermes-weekly-recap',
  },
  {
    id: 'cdx-onb',
    name: 'Codex . Onboarding draft',
    harness: 'Codex',
    loginScopeLabel: 'Selective (3)',
    loginCount: 3,
    aclRuleCount: 4,
    blockedActionCount: 1,
    alwaysAllowCount: 0,
    lastRunAt: 'Never run',
    status: 'disabled',
    mcpUrl: 'http://127.0.0.1:9000/mcp/codex-onboarding-draft',
  },
]

export const useAgentProfiles = createQuery<AgentProfile[]>({
  queryKey: ['agent-profiles'],
  fetcher: () =>
    new Promise((resolve) =>
      setTimeout(() => resolve(MOCK_AGENT_PROFILES), 60),
    ),
})

interface DeleteAgentVariables {
  id: string
}

/**
 * Mock deleteAgent mutation. The hono-rpc surface will return
 * `{ id }`; mutating clients update the `agent-profiles` cache with
 * `setQueryData` instead of refetching since the row goes away on
 * success.
 */
export const useDeleteAgent = createMutation<
  DeleteAgentVariables,
  DeleteAgentVariables
>({
  mutationFn: async (variables) => {
    await new Promise((resolve) => setTimeout(resolve, 400))
    return variables
  },
})

interface RegenerateMcpVariables {
  id: string
}

interface RegenerateMcpResult {
  id: string
  mcpUrl: string
}

/**
 * Mock mutation that rotates a profile's MCP URL. Shape matches the
 * eventual hono-rpc surface: server picks a fresh slug, returns the
 * new URL, the client updates the `agent-profiles` cache row by id.
 * The user must re-paste the URL into their harness once this fires.
 */
export const useRegenerateMcpUrl = createMutation<
  RegenerateMcpResult,
  RegenerateMcpVariables
>({
  mutationFn: async ({ id }) => {
    await new Promise((resolve) => setTimeout(resolve, 500))
    return {
      id,
      mcpUrl: `${buildMcpUrl(`${toSlug(id)}-${nanoid(6).toLowerCase()}`)}`,
    }
  },
})

interface UseAgentProfileDetailVariables {
  id: string
}

/**
 * Derive a full NewAgentValues from a profile summary. The directory
 * row carries summary fields (loginCount, aclRuleCount,
 * blockedActionCount); the wizard wants the full schema. The real
 * backend will persist + return the wizard shape directly; this
 * mock synthesises a plausible detail from the summary fields so the
 * edit screen prefills sensibly.
 */
function profileToWizardValues(profile: AgentProfile): NewAgentValues {
  const loginMode = profile.loginScopeLabel.startsWith('Selective')
    ? ('selective' as LoginMode)
    : profile.loginScopeLabel.startsWith('All my logins')
      ? ('all' as LoginMode)
      : ('profile' as LoginMode)
  const selectedSites =
    loginMode === 'selective'
      ? IMPORTED_SITES.slice(
          0,
          Math.min(profile.loginCount, IMPORTED_SITES.length),
        )
      : []
  const approvals = Object.fromEntries(
    APPROVAL_CATEGORIES.map((category) => [
      category.id,
      category.defaultVerdict,
    ]),
  ) as Record<string, ApprovalVerdict>
  const aclRuleIds = SEED_ACL_RULES.slice(
    0,
    Math.min(profile.aclRuleCount, SEED_ACL_RULES.length),
  ).map((rule) => rule.id)
  return {
    name: profile.name,
    harness: profile.harness,
    loginMode,
    selectedSites: [...selectedSites],
    approvals,
    aclRuleIds,
    customAclRules: [],
  }
}

export const useAgentProfileDetail = createQuery<
  NewAgentValues | null,
  UseAgentProfileDetailVariables
>({
  queryKey: ['agent-profile-detail'],
  fetcher: ({ id }) =>
    new Promise((resolve) =>
      setTimeout(() => {
        const profile = MOCK_AGENT_PROFILES.find((p) => p.id === id) ?? null
        resolve(profile ? profileToWizardValues(profile) : null)
      }, 80),
    ),
})

interface UpdateAgentVariables extends NewAgentValues {
  id: string
}

/**
 * Mock updateAgent mutation. Returns the updated values so the
 * caller can rewrite the matching agent-profiles row. Surface area
 * mirrors the eventual hono-rpc `PATCH /agents/:id` route.
 */
export const useUpdateAgent = createMutation<
  UpdateAgentVariables,
  UpdateAgentVariables
>({
  mutationFn: async (variables) => {
    await new Promise((resolve) => setTimeout(resolve, 500))
    return variables
  },
})
