/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */
import type { ToolApprovalConfig } from '@browseros/shared/constants/tool-approval'
import type { LLMProvider } from '@browseros/shared/schemas/llm'

export interface CompactionStrategyConfig {
  /** Compaction method. Default: "default" (LLM summarization) */
  method: 'default' | 'vcc'

  /**
   * When method=="default": replace the summarization prompt.
   * null/undefined = use built-in BrowserOS prompt.
   */
  customPrompt?: string

  /**
   * When method=="vcc": override section caps.
   * null/undefined = use built-in pi-vcc defaults.
   */
  vccConfig?: {
    maxTranscriptLines?: number
    maxGoalLines?: number
    maxFileEntries?: number
    maxCommitEntries?: number
    maxPreferenceLines?: number
    maxOutstandingLines?: number
  }
}

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
  /** Tool approval configuration — which categories require human approval. */
  toolApprovalConfig?: ToolApprovalConfig
  /** Compaction strategy configuration. Undefined = default LLM summarization. */
  compaction?: CompactionStrategyConfig
}
