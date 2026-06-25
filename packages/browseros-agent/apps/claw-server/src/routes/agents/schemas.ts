/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Zod shapes for the /agents routes. The UI's wizard-side validation
 * lives in `apps/claw-app/screens/new-agent/new-agent.schemas.ts`
 * (it needs zod for client-side form errors); these schemas are the
 * wire contract the typed client picks up via AppType.
 *
 * Storage shape extends the wire shape with server-managed fields
 * (id, slug, mcpUrl, status, timestamps). The directory's projection
 * shape lives at the bottom and is derived from the storage shape.
 */

import { z } from 'zod'

/**
 * The first 7 entries align 1:1 with `agent-mcp-manager`'s AgentId
 * space. The last 2 are BrowserOS-internal harnesses with no
 * third-party config to write — they short-circuit as a no-op
 * inside `services/harness-install`. Keep these in sync with
 * apps/claw-app/screens/new-agent/new-agent.schemas.ts.
 */
export const harnessEnum = z.enum([
  'Claude Code',
  'Claude Desktop',
  'Cursor',
  'VS Code',
  'Zed',
  'Codex',
  'Gemini CLI',
  'Hermes',
  'OpenClaw',
])
export type Harness = z.infer<typeof harnessEnum>

export const loginModeEnum = z.enum(['profile', 'all', 'selective'])
export type LoginMode = z.infer<typeof loginModeEnum>

export const approvalVerdictEnum = z.enum(['Auto', 'Ask', 'Block'])
export type ApprovalVerdict = z.infer<typeof approvalVerdictEnum>

export const profileStatusEnum = z.enum(['configured', 'paused', 'disabled'])
export type ProfileStatus = z.infer<typeof profileStatusEnum>

export const customAclRuleSchema = z.object({
  id: z.string(),
  label: z.string().min(1),
  domain: z.string().min(1),
})
export type CustomAclRule = z.infer<typeof customAclRuleSchema>

/** Wire shape: POST / PATCH body, also GET /:id response. Mirrors UI's NewAgentValues. */
export const newAgentValuesSchema = z.object({
  name: z.string().trim().min(1),
  harness: harnessEnum,
  loginMode: loginModeEnum,
  selectedSites: z.array(z.string()),
  approvals: z.record(z.string(), approvalVerdictEnum),
  aclRuleIds: z.array(z.string()),
  customAclRules: z.array(customAclRuleSchema),
})
export type NewAgentValues = z.infer<typeof newAgentValuesSchema>

/** On-disk shape under <browserosDir>/claw-server/agents/<id>.json. */
export const storedAgentProfileSchema = newAgentValuesSchema.extend({
  id: z.string(),
  slug: z.string(),
  mcpUrl: z.string(),
  status: profileStatusEnum,
  createdAt: z.string(),
  updatedAt: z.string(),
})
export type StoredAgentProfile = z.infer<typeof storedAgentProfileSchema>

/** Wire shape: GET / response. Directory row. */
export const agentProfileSummarySchema = z.object({
  id: z.string(),
  name: z.string(),
  harness: harnessEnum,
  loginScopeLabel: z.string(),
  loginCount: z.number(),
  aclRuleCount: z.number(),
  blockedActionCount: z.number(),
  alwaysAllowCount: z.number(),
  lastRunAt: z.string(),
  status: profileStatusEnum,
  mcpUrl: z.string(),
})
export type AgentProfileSummary = z.infer<typeof agentProfileSummarySchema>

/**
 * Outcome of the harness MCP install side-effect that runs alongside
 * create. The wizard surfaces `installed` and `message` directly; the
 * `configPath` is filled when a real file was written so the cockpit
 * can hint at where the entry landed.
 */
export const harnessInstallOutcomeSchema = z.object({
  installed: z.boolean(),
  message: z.string(),
  configPath: z.string().optional(),
})
export type HarnessInstallOutcome = z.infer<typeof harnessInstallOutcomeSchema>

/** Wire shape: POST / response. Carries the rail's display strings. */
export const createdAgentSchema = z.object({
  id: z.string(),
  name: z.string(),
  harness: harnessEnum,
  slug: z.string(),
  mcpUrl: z.string(),
  cliCommand: z.string(),
  harnessInstall: harnessInstallOutcomeSchema,
})
export type CreatedAgent = z.infer<typeof createdAgentSchema>

/** Wire shape: PATCH / response. */
export const updatedAgentSchema = storedAgentProfileSchema
export type UpdatedAgent = z.infer<typeof updatedAgentSchema>

/** Wire shape: DELETE / and regenerate. */
export const idAckSchema = z.object({ id: z.string() })
export type IdAck = z.infer<typeof idAckSchema>

/** Wire shape: DELETE / response — carries the uninstall side-effect. */
export const deletedAgentSchema = z.object({
  id: z.string(),
  harnessUninstall: harnessInstallOutcomeSchema,
})
export type DeletedAgent = z.infer<typeof deletedAgentSchema>

export const regeneratedMcpUrlSchema = z.object({
  id: z.string(),
  mcpUrl: z.string(),
})
export type RegeneratedMcpUrl = z.infer<typeof regeneratedMcpUrlSchema>
