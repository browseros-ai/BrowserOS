import { z } from 'zod/v3'
import { isLocalRuntimeProviderType } from '../../lib/llm-providers/provider-runtime'

const providerTypeEnum = z.enum([
  'moonshot',
  'anthropic',
  'openai',
  'openai-compatible',
  'google',
  'openrouter',
  'azure',
  'ollama',
  'lmstudio',
  'bedrock',
  'browseros',
  'chatgpt-pro',
  'github-copilot',
  'qwen-code',
  'codex',
  'claude-code',
  'acp-custom',
])

const credentiallessProviderTypes: ReadonlySet<
  z.infer<typeof providerTypeEnum>
> = new Set([
  'chatgpt-pro',
  'github-copilot',
  'qwen-code',
  'codex',
  'claude-code',
  'acp-custom',
])

export const providerFormSchema = z
  .object({
    type: providerTypeEnum,
    name: z.string().min(1, 'Provider name is required').max(50),
    baseUrl: z.string().optional(),
    modelId: z.string().min(1, 'Model ID is required'),
    apiKey: z.string().optional(),
    supportsImages: z.boolean(),
    contextWindow: z.number().int().min(1000).max(2000000),
    temperature: z.number().min(0).max(2),
    resourceName: z.string().optional(),
    accessKeyId: z.string().optional(),
    secretAccessKey: z.string().optional(),
    region: z.string().optional(),
    sessionToken: z.string().optional(),
    reasoningEffort: z
      .enum(['none', 'low', 'medium', 'high', 'xhigh', 'max'])
      .optional(),
    reasoningSummary: z.enum(['auto', 'concise', 'detailed']).optional(),
    acpAgentId: z.string().optional(),
    acpCommand: z.string().optional(),
    acpFixedWorkspacePath: z.string().optional(),
  })
  .superRefine((data, ctx) => {
    if (data.type === 'azure') {
      if (!data.resourceName && !data.baseUrl) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'Either Resource Name or Base URL is required',
          path: ['resourceName'],
        })
      }
      if (!data.apiKey) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'API Key is required for Azure',
          path: ['apiKey'],
        })
      }
    } else if (data.type === 'bedrock') {
      if (!data.accessKeyId) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'Access Key ID is required',
          path: ['accessKeyId'],
        })
      }
      if (!data.secretAccessKey) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'Secret Access Key is required',
          path: ['secretAccessKey'],
        })
      }
      if (!data.region) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'Region is required',
          path: ['region'],
        })
      }
    } else if (credentiallessProviderTypes.has(data.type)) {
      return
    } else if (!data.baseUrl) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Base URL is required',
        path: ['baseUrl'],
      })
    } else if (!/^https?:\/\/.+/.test(data.baseUrl)) {
      ctx.addIssue({
        code: z.ZodIssueCode.custom,
        message: 'Must be a valid URL',
        path: ['baseUrl'],
      })
    }
  })

export type ProviderFormValues = z.infer<typeof providerFormSchema>

/** Identifies provider types whose settings form does not collect credentials. */
export function isCredentiallessProviderType(
  type: z.infer<typeof providerTypeEnum>,
): boolean {
  return credentiallessProviderTypes.has(type)
}

/** Removes stale endpoint and credential fields from local runtime configs. */
export function normalizeProviderFormValues(
  values: ProviderFormValues,
): ProviderFormValues {
  if (!isLocalRuntimeProviderType(values.type)) return values

  return {
    ...values,
    baseUrl: '',
    apiKey: '',
    resourceName: '',
    accessKeyId: '',
    secretAccessKey: '',
    region: '',
    sessionToken: '',
  }
}
