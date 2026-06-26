import { z } from 'zod'

/**
 * The first 7 entries align 1:1 with `agent-mcp-manager`'s AgentId
 * space and trigger a real config write on create. The last 2 are
 * BrowserOS-internal harnesses that no-op the install. Keep this
 * list in sync with the backend's harnessEnum at
 * `apps/claw-server/src/routes/agents/schemas.ts`.
 *
 * NOTE: `HARNESSES` is the full Harness type-domain — it covers any
 * harness value we may have stored historically. The wizard picker
 * iterates `SELECTABLE_HARNESSES` (below) instead, which subtracts
 * `RETIRED_HARNESSES` to drop options BrowserOS no longer offers for
 * new creates. Existing profiles whose harness lives in
 * `RETIRED_HARNESSES` still parse, render with the right icon, and
 * can be uninstalled — they just can't be re-picked.
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

/**
 * Harnesses removed from the new-agent picker. Claude Desktop is
 * hidden because its config parser only validates stdio entries and
 * the recommended `npx mcp-remote` bridge requires Node on the
 * user's machine, which BrowserOS cannot guarantee. Mirrors the
 * apps/server `HIDDEN_AGENTS` rationale.
 */
export const RETIRED_HARNESSES = [
  'Claude Desktop',
] as const satisfies readonly Harness[]

export const SELECTABLE_HARNESSES = HARNESSES.filter(
  (h): h is Exclude<Harness, (typeof RETIRED_HARNESSES)[number]> =>
    !(RETIRED_HARNESSES as readonly Harness[]).includes(h),
)

export type SelectableHarness = (typeof SELECTABLE_HARNESSES)[number]

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
  // Form validation rejects retired harnesses so a hand-crafted
  // submission can't slip Claude Desktop back into the create path.
  harness: z.enum(SELECTABLE_HARNESSES),
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
