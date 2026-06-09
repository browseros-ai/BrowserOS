/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */
import type { LLMProvider } from '@browseros/shared/schemas/llm'
import type { McpServerSpec } from '../lib/agents/acpx-provider/buildAcpxProvider'

export interface ProviderConfig {
  provider: LLMProvider
  model: string
  apiKey?: string
  baseUrl?: string
  upstreamProvider?: string
  resourceName?: string
  region?: string
  accessKeyId?: string
  secretAccessKey?: string
  sessionToken?: string
}

export interface ResolvedAgentConfig {
  conversationId: string
  provider: LLMProvider
  model: string
  apiKey?: string
  baseUrl?: string
  upstreamProvider?: string
  resourceName?: string
  region?: string
  accessKeyId?: string
  secretAccessKey?: string
  sessionToken?: string
  accountId?: string
  reasoningEffort?: string
  reasoningSummary?: string
  contextWindowSize?: number
  userSystemPrompt?: string
  workingDir?: string
  /** Whether the model supports image inputs (vision). Defaults to true. */
  supportsImages?: boolean
  /** Eval mode - enables window management tools. Defaults to false. */
  evalMode?: boolean
  /** Chat mode - restricts to read-only tools (no browser automation). Defaults to false. */
  chatMode?: boolean
  /** Scheduled task mode - disables tab grouping. Defaults to false. */
  isScheduledTask?: boolean
  /** Apps the user previously declined to connect via MCP (chose "do it manually"). */
  declinedApps?: string[]
  /** Where the chat session originates from — determines navigation behavior. */
  origin?: 'sidepanel' | 'newtab'
  /** BrowserOS installation ID for credit-based tracking. */
  browserosId?: string

  /** ACP agent id (claude / codex / custom registry name). Only set
   *  when provider is one of the ACP-backed types. */
  acpAgentId?: string
  /** Shell command for the spawned ACP agent. Only set when provider
   *  is 'acp-custom'; built-in agents resolve through acpx's registry. */
  acpCommand?: string
  /** Fixed cwd the user picked at provider-create time. Used as-is for
   *  ACP-backed providers; ignored for model-backed ones. */
  acpFixedWorkspacePath?: string
  /** MCP servers exposed to the spawned ACP agent. Computed at request
   *  time from BrowserOS's own /mcp URL plus the user's custom MCP
   *  servers in browserContext. Only consumed by the ACP factory
   *  branch; model-backed factories ignore it. */
  acpMcpServers?: McpServerSpec[]
}
