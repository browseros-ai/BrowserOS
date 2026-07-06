/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
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

const loginModeEnum = z.enum(['profile', 'all', 'selective'])

const approvalVerdictEnum = z.enum(['Auto', 'Ask', 'Block'])

const profileStatusEnum = z.enum(['configured', 'paused', 'disabled'])

const customAclRuleSchema = z.object({
  id: z.string(),
  label: z.string().min(1),
  domain: z.string().min(1),
})

const newAgentValuesSchema = z.object({
  name: z.string().trim().min(1),
  harness: harnessEnum,
  loginMode: loginModeEnum,
  selectedSites: z.array(z.string()),
  approvals: z.record(z.string(), approvalVerdictEnum),
  aclRuleIds: z.array(z.string()),
  customAclRules: z.array(customAclRuleSchema),
})
export type NewAgentValues = z.infer<typeof newAgentValuesSchema>

export const storedAgentProfileSchema = newAgentValuesSchema.extend({
  id: z.string(),
  slug: z.string(),
  mcpUrl: z.string(),
  status: profileStatusEnum,
  createdAt: z.string(),
  updatedAt: z.string(),
})
export type StoredAgentProfile = z.infer<typeof storedAgentProfileSchema>

const agentProfileSummarySchema = z.object({
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

const harnessInstallOutcomeSchema = z.object({
  installed: z.boolean(),
  message: z.string(),
  configPath: z.string().optional(),
})

const createdAgentSchema = z.object({
  id: z.string(),
  name: z.string(),
  harness: harnessEnum,
  slug: z.string(),
  mcpUrl: z.string(),
  cliCommand: z.string(),
  harnessInstall: harnessInstallOutcomeSchema,
})
export type CreatedAgent = z.infer<typeof createdAgentSchema>

const deletedAgentSchema = z.object({
  id: z.string(),
  harnessUninstall: harnessInstallOutcomeSchema,
})
export type DeletedAgent = z.infer<typeof deletedAgentSchema>

const regeneratedMcpUrlSchema = z.object({
  id: z.string(),
  mcpUrl: z.string(),
})
export type RegeneratedMcpUrl = z.infer<typeof regeneratedMcpUrlSchema>
