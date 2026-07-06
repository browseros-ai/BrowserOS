/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Zod shapes for stored agent profiles. The live MCP permission
 * chain reads these profiles directly; no Agent CRUD HTTP route is
 * mounted.
 */

import { z } from 'zod'

/**
 * The first 7 entries align 1:1 with `agent-mcp-manager`'s AgentId
 * space. The last 2 are BrowserOS-internal harnesses with no
 * third-party config to write — they short-circuit as a no-op
 * inside `services/harness-install`. Keep these in sync with
 * apps/claw-app/components/harness/harness.types.ts.
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

/** Editable profile fields stored before server-managed metadata is added. */
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

/** Directory projection used by tabs/activity enrichment. */
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
 * Outcome of harness MCP install/uninstall side effects.
 */
export const harnessInstallOutcomeSchema = z.object({
  installed: z.boolean(),
  message: z.string(),
  configPath: z.string().optional(),
})
export type HarnessInstallOutcome = z.infer<typeof harnessInstallOutcomeSchema>

/** Result returned by the agent-profile create service. */
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

/** Result returned by the agent-profile update service. */
export const updatedAgentSchema = storedAgentProfileSchema
export type UpdatedAgent = z.infer<typeof updatedAgentSchema>

/** Acknowledges a service mutation by id. */
export const idAckSchema = z.object({ id: z.string() })
export type IdAck = z.infer<typeof idAckSchema>

/** Result returned by the agent-profile delete service. */
export const deletedAgentSchema = z.object({
  id: z.string(),
  harnessUninstall: harnessInstallOutcomeSchema,
})
export type DeletedAgent = z.infer<typeof deletedAgentSchema>

/** Result returned by MCP URL regeneration. */
export const regeneratedMcpUrlSchema = z.object({
  id: z.string(),
  mcpUrl: z.string(),
})
export type RegeneratedMcpUrl = z.infer<typeof regeneratedMcpUrlSchema>
