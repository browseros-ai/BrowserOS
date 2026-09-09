import {
  type LLMConfig,
  LLMProviderSchema,
} from '@browseros/shared/schemas/llm'
import type { ProviderRow } from '../db/schema'
import type { ProviderUpsert } from './provider-store'

/** Provider connection defaults belong to the server, including legacy reads. */
export const PROVIDER_DEFAULT_URLS: Readonly<Record<string, string>> = {
  openai: 'https://api.openai.com/v1',
  anthropic: 'https://api.anthropic.com/v1',
  google: 'https://generativelanguage.googleapis.com/v1beta',
  openrouter: 'https://openrouter.ai/api/v1',
  moonshot: 'https://api.moonshot.ai/v1',
  ollama: 'http://localhost:11434/v1',
  lmstudio: 'http://localhost:1234/v1',
  'chatgpt-pro': 'https://chatgpt.com/backend-api',
  'github-copilot': 'https://api.githubcopilot.com',
  'qwen-code': 'https://portal.qwen.ai/v1',
}

export const SINGLE_INSTANCE_PROVIDERS = new Set([
  'chatgpt-pro',
  'github-copilot',
  'qwen-code',
])

const CREDENTIAL_FIELDS = [
  'apiKey',
  'accessKeyId',
  'secretAccessKey',
  'sessionToken',
] as const
const API_KEY_PROVIDERS = new Set([
  'openai',
  'anthropic',
  'google',
  'openrouter',
  'moonshot',
  'azure',
])

/** Field errors cross the HTTP boundary without exposing configuration or secrets. */
export class ProviderConfigError extends Error {
  constructor(readonly fieldErrors: Record<string, string>) {
    super(Object.values(fieldErrors).join(' '))
    this.name = 'ProviderConfigError'
  }
}

type Destination = Pick<
  ProviderUpsert,
  'type' | 'baseUrl' | 'resourceName' | 'region'
>

export function providerBaseUrl(
  config: Pick<Destination, 'type' | 'baseUrl'>,
): string {
  return config.baseUrl || PROVIDER_DEFAULT_URLS[config.type] || ''
}

function canonicalUrl(value: string): string {
  if (!value) return ''
  try {
    const url = new URL(value)
    if (url.protocol !== 'http:' && url.protocol !== 'https:') throw new Error()
    // SDKs append request paths to this base. A trailing slash and an implicit
    // provider default must not make an unchanged endpoint look like a new one.
    url.pathname = url.pathname.replace(/\/$/, '')
    return url.toString().replace(/\/$/, '')
  } catch {
    throw new ProviderConfigError({
      baseUrl: 'Enter a valid HTTP or HTTPS base URL.',
    })
  }
}

function destination(config: Destination): string {
  return JSON.stringify([
    config.type,
    canonicalUrl(providerBaseUrl(config)),
    config.type === 'azure' && !config.baseUrl ? config.resourceName || '' : '',
    config.type === 'bedrock' ? config.region || '' : '',
  ])
}

function matchesSavedDestination(
  config: Destination,
  saved: Destination,
): boolean {
  const target = destination(config)
  try {
    return target === destination(saved)
  } catch (error) {
    if (!(error instanceof ProviderConfigError)) throw error
    // Older versions accepted malformed URLs. Let users repair those rows,
    // but never carry credentials from an endpoint we cannot identify.
    return false
  }
}

/**
 * Resolves a provider draft for both save and test. Clients never need a secret
 * to edit a saved provider: blank retains it only at the same effective
 * destination. A changed destination cannot borrow a saved credential.
 */
export function resolveProviderConfig(
  changes: ProviderUpsert,
  saved: ProviderRow | null = null,
): ProviderUpsert {
  if (!LLMProviderSchema.safeParse(changes.type).success) {
    throw new ProviderConfigError({ type: 'Choose a supported provider type.' })
  }
  const supplied = Object.fromEntries(
    Object.entries(changes).filter(([, value]) => value !== undefined),
  )
  const config: ProviderUpsert = {
    ...(saved?.type === changes.type ? saved : {}),
    ...supplied,
  } as ProviderUpsert
  config.baseUrl = canonicalUrl(providerBaseUrl(config)) || null
  const sameDestination =
    saved !== null && matchesSavedDestination(config, saved)

  for (const field of CREDENTIAL_FIELDS) {
    // null explicitly clears a credential; '' and undefined preserve it.
    config[field] =
      changes[field] === null
        ? null
        : changes[field] || (sameDestination ? saved?.[field] : null) || null
  }

  const errors: Record<string, string> = {}
  const require = (field: keyof ProviderUpsert, message: string) => {
    if (
      !config[field] ||
      (typeof config[field] === 'string' && !config[field].trim())
    )
      errors[field] = message
  }
  require('modelId', 'Enter a model ID.')
  if (API_KEY_PROVIDERS.has(config.type))
    require('apiKey', 'Enter an API key for this provider.')
  if (config.type === 'azure' && !config.baseUrl)
    require('resourceName', 'Enter an Azure resource name or base URL.')
  if (config.type === 'openai-compatible')
    require('baseUrl', 'Enter the base URL of your compatible provider.')
  if (config.type === 'bedrock') {
    require('region', 'Enter an AWS region.')
    require('accessKeyId', 'Enter an AWS access key ID.')
    require('secretAccessKey', 'Enter an AWS secret access key.')
  }
  for (const field of CREDENTIAL_FIELDS) {
    if (
      errors[field] &&
      saved?.[field] &&
      !sameDestination &&
      !changes[field]
    ) {
      errors[field] =
        'The provider destination changed. Enter credentials for the new destination.'
    }
  }
  if (Object.keys(errors).length) throw new ProviderConfigError(errors)
  return config
}

/** Translates the resolved saved/draft configuration for the connection test. */
export function providerToLlmConfig(
  row: ProviderUpsert,
): LLMConfig & { model: string } {
  return {
    provider: row.type as LLMConfig['provider'],
    providerId: row.id,
    model: row.modelId ?? '',
    baseUrl: providerBaseUrl(row) || undefined,
    headers: row.headers ?? undefined,
    apiKey: row.apiKey ?? undefined,
    resourceName: row.resourceName ?? undefined,
    region: row.region ?? undefined,
    accessKeyId: row.accessKeyId ?? undefined,
    secretAccessKey: row.secretAccessKey ?? undefined,
    sessionToken: row.sessionToken ?? undefined,
    reasoningEffort: row.reasoningEffort as LLMConfig['reasoningEffort'],
    reasoningSummary: row.reasoningSummary as LLMConfig['reasoningSummary'],
  }
}
