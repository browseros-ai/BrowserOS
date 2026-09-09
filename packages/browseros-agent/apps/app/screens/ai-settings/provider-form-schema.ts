import {
  HEADER_NAME_PATTERN,
  HEADER_VALUE_PATTERN,
} from '@browseros/shared/schemas/llm'
import { z } from 'zod/v3'

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
])

const credentiallessProviderTypes: ReadonlySet<
  z.infer<typeof providerTypeEnum>
> = new Set(['chatgpt-pro', 'github-copilot', 'qwen-code'])

// Validate editor structure here. The server resolves connection defaults and
// validates credentials for both Save and Test, using the same saved record.
export const providerFormSchema = z.object({
  type: providerTypeEnum,
  name: z.string().min(1, 'Provider name is required').max(50),
  baseUrl: z.string().optional(),
  headers: z
    .array(
      z.object({
        name: z
          .string()
          .regex(HEADER_NAME_PATTERN, 'Enter a valid HTTP header name'),
        value: z
          .string()
          .regex(
            HEADER_VALUE_PATTERN,
            'Header values cannot contain newlines or unsupported characters',
          ),
      }),
    )
    .superRefine((headers, ctx) => {
      const names = new Set<string>()
      headers.forEach(({ name }, index) => {
        const normalized = name.toLowerCase()
        if (names.has(normalized)) {
          ctx.addIssue({
            code: z.ZodIssueCode.custom,
            message: 'Duplicate header name',
            path: [index, 'name'],
          })
        }
        names.add(normalized)
      })
    })
    .optional(),
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
  reasoningEffort: z.string().optional(),
  reasoningSummary: z.enum(['auto', 'concise', 'detailed']).optional(),
})

export type ProviderFormValues = z.infer<typeof providerFormSchema>

/** Identifies provider types whose settings form does not collect credentials. */
export function isCredentiallessProviderType(
  type: z.infer<typeof providerTypeEnum>,
): boolean {
  return credentiallessProviderTypes.has(type)
}

export function normalizeProviderFormValues(
  values: ProviderFormValues,
): Omit<ProviderFormValues, 'headers'> & { headers?: Record<string, string> } {
  const { headers, ...rest } = values
  return {
    ...rest,
    ...(headers && {
      headers: Object.fromEntries(
        headers.map(({ name, value }) => [name, value]),
      ),
    }),
  }
}
