/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Shared LLM configuration Zod schemas - single source of truth.
 * Use z.infer<> for TypeScript types.
 */

import { z } from 'zod'

/**
 * LLM provider constants for type-safe switch statements
 */
export const LLM_PROVIDERS = {
  ANTHROPIC: 'anthropic',
  OPENAI: 'openai',
  GOOGLE: 'google',
  OPENROUTER: 'openrouter',
  AZURE: 'azure',
  OLLAMA: 'ollama',
  LMSTUDIO: 'lmstudio',
  BEDROCK: 'bedrock',
  BROWSEROS: 'browseros',
  OPENAI_COMPATIBLE: 'openai-compatible',
  MOONSHOT: 'moonshot',
  CHATGPT_PRO: 'chatgpt-pro',
  GITHUB_COPILOT: 'github-copilot',
  QWEN_CODE: 'qwen-code',
  CLAUDE_CODE: 'claude-code',
  CODEX: 'codex',
  ACP_CUSTOM: 'acp-custom',
} as const

/**
 * Supported LLM providers
 */
export const LLMProviderSchema: z.ZodEnum<
  [
    'anthropic',
    'openai',
    'google',
    'openrouter',
    'azure',
    'ollama',
    'lmstudio',
    'bedrock',
    'browseros',
    'openai-compatible',
    'moonshot',
    'chatgpt-pro',
    'github-copilot',
    'qwen-code',
    'claude-code',
    'codex',
    'acp-custom',
  ]
> = z.enum([
  LLM_PROVIDERS.ANTHROPIC,
  LLM_PROVIDERS.OPENAI,
  LLM_PROVIDERS.GOOGLE,
  LLM_PROVIDERS.OPENROUTER,
  LLM_PROVIDERS.AZURE,
  LLM_PROVIDERS.OLLAMA,
  LLM_PROVIDERS.LMSTUDIO,
  LLM_PROVIDERS.BEDROCK,
  LLM_PROVIDERS.BROWSEROS,
  LLM_PROVIDERS.OPENAI_COMPATIBLE,
  LLM_PROVIDERS.MOONSHOT,
  LLM_PROVIDERS.CHATGPT_PRO,
  LLM_PROVIDERS.GITHUB_COPILOT,
  LLM_PROVIDERS.QWEN_CODE,
  LLM_PROVIDERS.CLAUDE_CODE,
  LLM_PROVIDERS.CODEX,
  LLM_PROVIDERS.ACP_CUSTOM,
])

export type LLMProvider = z.infer<typeof LLMProviderSchema>

/**
 * LLM configuration schema
 * Used by SDK endpoints and agent configuration
 */
export const LLMConfigSchema: z.ZodObject<{
  provider: typeof LLMProviderSchema
  providerId: z.ZodOptional<z.ZodString>
  model: z.ZodOptional<z.ZodString>
  apiKey: z.ZodOptional<z.ZodString>
  baseUrl: z.ZodOptional<z.ZodString>
  resourceName: z.ZodOptional<z.ZodString>
  region: z.ZodOptional<z.ZodString>
  accessKeyId: z.ZodOptional<z.ZodString>
  secretAccessKey: z.ZodOptional<z.ZodString>
  sessionToken: z.ZodOptional<z.ZodString>
  reasoningEffort: z.ZodOptional<
    z.ZodEnum<['none', 'low', 'medium', 'high', 'xhigh', 'max']>
  >
  reasoningSummary: z.ZodOptional<z.ZodEnum<['auto', 'concise', 'detailed']>>
  acpAgentId: z.ZodOptional<z.ZodString>
  acpCommand: z.ZodOptional<z.ZodString>
  acpFixedWorkspacePath: z.ZodOptional<z.ZodString>
}> = z.object({
  provider: LLMProviderSchema,
  // The unique LlmProviderConfig.id this request points at. Used by the
  // ACP factory to scope the default workspace path so two providers of
  // the same type (e.g. Claude Opus High vs Claude Sonnet Medium) get
  // their own isolated working directory instead of sharing one.
  providerId: z.string().optional(),
  model: z.string().optional(),
  apiKey: z.string().optional(),
  baseUrl: z.string().optional(),
  // Azure-specific
  resourceName: z.string().optional(),
  // AWS Bedrock-specific
  region: z.string().optional(),
  accessKeyId: z.string().optional(),
  secretAccessKey: z.string().optional(),
  sessionToken: z.string().optional(),
  // ChatGPT Pro (Codex) shares the lower half; ACP-backed providers
  // (claude advertises xhigh + max) extend it further. The wider enum
  // accepts every value any ACP agent emits so the chat path can pass
  // probe-discovered values through verbatim.
  reasoningEffort: z
    .enum(['none', 'low', 'medium', 'high', 'xhigh', 'max'])
    .optional(),
  reasoningSummary: z.enum(['auto', 'concise', 'detailed']).optional(),
  // ACP-backed providers (claude-code, codex, acp-custom). agent id
  // resolves through acpx's registry; command is only used when
  // provider is 'acp-custom'; workspace is the fixed-path cwd the
  // user picks at provider-create time.
  acpAgentId: z.string().optional(),
  acpCommand: z.string().optional(),
  acpFixedWorkspacePath: z.string().optional(),
})

export type LLMConfig = z.infer<typeof LLMConfigSchema>
