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
} as const

/**
 * Supported LLM providers
 */
export const LLMProviderSchema = z.enum([
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
])

export type LLMProvider = z.infer<typeof LLMProviderSchema>

export const CONVERSATION_ID_PLACEHOLDER = '{{conversationId}}'
// Unlike $, the final assertion rejects trailing newlines too.
export const HEADER_NAME_PATTERN = /^[!#$%&'*+.^_`|~0-9A-Za-z-]+(?![\s\S])/
export const HEADER_VALUE_PATTERN = /^[\t\x20-\x7e\x80-\xff]*(?![\s\S])/

export const LLMHeadersSchema = z
  .record(
    z.string().regex(HEADER_NAME_PATTERN),
    z.string().regex(HEADER_VALUE_PATTERN),
  )
  .superRefine((headers, ctx) => {
    const names = new Set<string>()
    for (const name of Object.keys(headers)) {
      const normalized = name.toLowerCase()
      if (names.has(normalized)) {
        ctx.addIssue({
          code: 'custom',
          message: 'Duplicate header name',
          path: [name],
        })
      }
      names.add(normalized)
    }
  })

/**
 * LLM configuration schema
 * Used by SDK endpoints and agent configuration
 */
export const LLMConfigSchema = z.object({
  provider: LLMProviderSchema,
  providerId: z.string().optional(),
  model: z.string().optional(),
  apiKey: z.string().optional(),
  baseUrl: z.string().optional(),
  headers: LLMHeadersSchema.optional(),
  // Azure-specific
  resourceName: z.string().optional(),
  // AWS Bedrock-specific
  region: z.string().optional(),
  accessKeyId: z.string().optional(),
  secretAccessKey: z.string().optional(),
  sessionToken: z.string().optional(),
  reasoningEffort: z
    .enum(['none', 'low', 'medium', 'high', 'xhigh', 'max'])
    .optional(),
  reasoningSummary: z.enum(['auto', 'concise', 'detailed']).optional(),
})

export type LLMConfig = z.infer<typeof LLMConfigSchema>
