import { z } from 'zod'

/**
 * The first 7 entries align 1:1 with `agent-mcp-manager`'s AgentId
 * space and trigger a real config write on create. The last 2 are
 * BrowserOS-internal harnesses that no-op the install. Keep this
 * list in sync with the backend's harnessEnum at
 * `apps/agent-mcp-interface/src/routes/agents/schemas.ts`.
 */
export const HARNESSES = [
  'Claude Code',
  'Claude Desktop',
  'Cursor',
  'VS Code',
  'Zed',
  'Codex',
  'Gemini CLI',
  'Hermes',
  'OpenClaw',
] as const

export type Harness = (typeof HARNESSES)[number]

export const LOGIN_MODES = ['profile', 'all', 'selective'] as const
export type LoginMode = (typeof LOGIN_MODES)[number]

export const APPROVAL_VERDICTS = ['Auto', 'Ask', 'Block'] as const
export type ApprovalVerdict = (typeof APPROVAL_VERDICTS)[number]

export interface ApprovalCategory {
  id: string
  name: string
  defaultVerdict: ApprovalVerdict
  allowAuto: boolean
}

export const APPROVAL_CATEGORIES: readonly ApprovalCategory[] = [
  {
    id: 'submit',
    name: 'Submit / send / post',
    defaultVerdict: 'Ask',
    allowAuto: true,
  },
  {
    id: 'payment',
    name: 'Payments & checkout',
    defaultVerdict: 'Block',
    allowAuto: false,
  },
  {
    id: 'delete',
    name: 'Delete / destructive',
    defaultVerdict: 'Ask',
    allowAuto: true,
  },
  { id: 'upload', name: 'File upload', defaultVerdict: 'Ask', allowAuto: true },
  {
    id: 'navigate',
    name: 'Navigate to a new site',
    defaultVerdict: 'Ask',
    allowAuto: true,
  },
  {
    id: 'input',
    name: 'Click & type',
    defaultVerdict: 'Auto',
    allowAuto: true,
  },
] as const

export const customAclRuleSchema = z.object({
  id: z.string(),
  label: z.string().min(1),
  domain: z.string().min(1),
})

export type CustomAclRule = z.infer<typeof customAclRuleSchema>

export const newAgentSchema = z.object({
  name: z.string().trim().min(1, 'Give the connector a name'),
  harness: z.enum(HARNESSES),
  loginMode: z.enum(LOGIN_MODES),
  selectedSites: z.array(z.string()),
  approvals: z.record(z.string(), z.enum(APPROVAL_VERDICTS)),
  aclRuleIds: z.array(z.string()),
  customAclRules: z.array(customAclRuleSchema),
})

export type NewAgentValues = z.infer<typeof newAgentSchema>

export const newAgentDefaults: NewAgentValues = {
  name: '',
  harness: 'Claude Code',
  loginMode: 'profile',
  selectedSites: ['concur.com', 'stripe.com'],
  approvals: Object.fromEntries(
    APPROVAL_CATEGORIES.map((c) => [c.id, c.defaultVerdict]),
  ) as Record<string, ApprovalVerdict>,
  aclRuleIds: [],
  customAclRules: [],
}
