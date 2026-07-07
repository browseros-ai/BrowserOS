/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { z } from 'zod'

/**
 * Every entry maps 1:1 to an agent-mcp-manager AgentId via
 * services/harness-install#HARNESS_TO_AGENT_ID. Keep in sync with
 * apps/claw-app/components/harness/harness.types.ts.
 */
export const harnessEnum = z.enum([
  'Claude Code',
  'Codex',
  'Cursor',
  'OpenCode',
  'Antigravity',
  'VS Code',
  'Zed',
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

const storedAgentProfileBaseSchema = z.object({
  name: z.string().trim().min(1),
  harness: harnessEnum,
  loginMode: loginModeEnum,
  selectedSites: z.array(z.string()),
  approvals: z.record(z.string(), approvalVerdictEnum),
  aclRuleIds: z.array(z.string()),
  customAclRules: z.array(customAclRuleSchema),
})

export const storedAgentProfileSchema = storedAgentProfileBaseSchema.extend({
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
