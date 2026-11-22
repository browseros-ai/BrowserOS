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
  'chatgpt-pro',
  'fireworks',
])

/** Where a provider sends its credential, as compared by `sameProviderAddress`. */
export interface ProviderAddress {
  type?: z.infer<typeof providerTypeEnum>
  baseUrl?: string
  resourceName?: string
  region?: string
}

type AddressField = Exclude<keyof ProviderAddress, 'type'>

function addressFields(type: ProviderAddress['type']): AddressField[] {
  if (type === 'bedrock') return ['region']
  if (type === 'azure') return ['baseUrl', 'resourceName']
  if (type === 'chatgpt-pro') return []
  return ['baseUrl']
}

const blankToUndefined = (value?: string) => (value?.trim() ? value : undefined)

/**
 * Whether two configs send their credential to the same address: the same type
 * and the same values in the fields that type sends its key to (Bedrock:
 * region; Azure: base URL and resource name; ChatGPT Pro: none; every other
 * type: base URL). A blank value equals an absent one, and anything else
 * compares exactly as written.
 *
 * Mirrors the server's save rule (crates/browseros-server/src/store/providers.rs):
 * a save that changes the address and brings no new secret clears the kept
 * one, so the form must stop offering "leave blank to keep" as soon as the
 * address differs from the stored row's.
 */
export function sameProviderAddress(
  current: ProviderAddress,
  original: ProviderAddress,
): boolean {
  if (current.type !== original.type) return false
  return addressFields(current.type).every(
    (field) =>
      blankToUndefined(current[field]) === blankToUndefined(original[field]),
  )
}

export const providerFormSchema = z
  .object({
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
    // Set when editing a provider that already has the credential stored, so a
    // blank field means "keep the saved value" rather than a missing
    // credential. Populated from the server's has* flags, never user-entered.
    hasApiKey: z.boolean().optional(),
    hasAccessKeyId: z.boolean().optional(),
    hasSecretAccessKey: z.boolean().optional(),
    // The stored row's address when the edit started, raw as read back (an
    // absent base URL stays undefined, not the form's default). A
    // stored-credential flag only applies while the address is unchanged:
    // the server clears a kept credential on a save that moves it.
    originalType: providerTypeEnum.optional(),
    originalBaseUrl: z.string().optional(),
    originalResourceName: z.string().optional(),
    originalRegion: z.string().optional(),
    reasoningEffort: z.string().optional(),
    reasoningSummary: z.enum(['auto', 'concise', 'detailed']).optional(),
  })
  .superRefine((data, ctx) => {
    const addressUnchanged = sameProviderAddress(data, {
      type: data.originalType,
      baseUrl: data.originalBaseUrl,
      resourceName: data.originalResourceName,
      region: data.originalRegion,
    })
    const hasStoredApiKey = Boolean(data.hasApiKey) && addressUnchanged
    const hasStoredAccessKeyId =
      Boolean(data.hasAccessKeyId) && addressUnchanged
    const hasStoredSecretAccessKey =
      Boolean(data.hasSecretAccessKey) && addressUnchanged
    if (data.type === 'azure') {
      if (!data.resourceName && !data.baseUrl) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'Either Resource Name or Base URL is required',
          path: ['resourceName'],
        })
      }
      if (!data.apiKey && !hasStoredApiKey) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'API Key is required for Azure',
          path: ['apiKey'],
        })
      }
    } else if (data.type === 'bedrock') {
      if (!data.accessKeyId && !hasStoredAccessKeyId) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'Access Key ID is required',
          path: ['accessKeyId'],
        })
      }
      if (!data.secretAccessKey && !hasStoredSecretAccessKey) {
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
    } else if (data.type === 'chatgpt-pro') {
      return
    } else {
      if (!data.baseUrl) {
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
      // A stored key that this save will clear is asked for again, so the
      // provider is not left keyless without the person noticing. The server
      // runs these three types without a key (a keyless gateway or a local
      // model), so for them a blank key stays a choice.
      const apiKeyOptional =
        data.type === 'openai-compatible' ||
        data.type === 'ollama' ||
        data.type === 'lmstudio'
      if (
        data.hasApiKey &&
        !hasStoredApiKey &&
        !data.apiKey &&
        !apiKeyOptional
      ) {
        ctx.addIssue({
          code: z.ZodIssueCode.custom,
          message: 'API Key is required',
          path: ['apiKey'],
        })
      }
    }
  })

export type ProviderFormValues = z.infer<typeof providerFormSchema>

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
