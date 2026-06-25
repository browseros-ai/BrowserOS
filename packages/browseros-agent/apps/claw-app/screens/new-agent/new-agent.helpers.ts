import type { LoginMode, NewAgentValues } from './new-agent.schemas'

export const IMPORTED_SITES = [
  'concur.com',
  'stripe.com',
  'ramp.com',
  'mail.google.com',
  'docs.google.com',
  'linkedin.com',
  'x.com',
  'app.hubspot.com',
  'salesforce.com',
  'amazon.com',
  'github.com',
  'notion.so',
] as const

export const TOTAL_PROFILE_LOGINS = 47

export interface AclRule {
  id: string
  label: string
  domain: string
  custom?: boolean
}

export const SEED_ACL_RULES: readonly AclRule[] = [
  { id: 'wire-transfers', label: 'Wire transfers', domain: 'mercury.com' },
  {
    id: 'payment-methods',
    label: 'Edit payment methods',
    domain: 'stripe.com',
  },
  { id: 'org-billing', label: 'Org billing settings', domain: 'admin.*' },
  {
    id: 'user-management',
    label: 'User management',
    domain: 'workspace.google.com',
  },
  { id: 'delete-account', label: 'Delete account', domain: '*' },
]

export function toSlug(input: string): string {
  const cleaned = input
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')
  return cleaned || 'agent'
}

// MCP endpoint URL + CLI command are sourced from
// `@/modules/api/mcp-endpoint` so the create wizard, the edit flow,
// and the directory all render an identical "copy URL" widget and
// identical host-agent config snippets. See that module's header for
// the temporary cockpit-mount caveat.
export {
  buildMcpCliCommand as buildCliCommand,
  buildMcpEndpointUrl as buildMcpUrl,
} from '@/modules/api/mcp-endpoint'

export function describeLogins(
  mode: LoginMode,
  selectedCount: number,
): { count: number; label: string } {
  if (mode === 'all') {
    return {
      count: TOTAL_PROFILE_LOGINS,
      label: `All ${TOTAL_PROFILE_LOGINS} logins`,
    }
  }
  if (mode === 'profile') {
    return {
      count: TOTAL_PROFILE_LOGINS,
      label: `Current profile (${TOTAL_PROFILE_LOGINS})`,
    }
  }
  return { count: selectedCount, label: `${selectedCount} selected` }
}

export function normalizeDomainInput(raw: string): string {
  return raw
    .trim()
    .replace(/^https?:\/\//, '')
    .replace(/\/.*$/, '')
}

export function countApprovalVerdicts(approvals: NewAgentValues['approvals']) {
  const values = Object.values(approvals)
  return {
    auto: values.filter((v) => v === 'Auto').length,
    ask: values.filter((v) => v === 'Ask').length,
    block: values.filter((v) => v === 'Block').length,
  }
}
