/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

export const SUPPORTED_OPENCLAW_PROVIDERS = [
  'openrouter',
  'openai',
  'anthropic',
  'moonshot',
] as const

export type SupportedOpenClawProvider =
  (typeof SUPPORTED_OPENCLAW_PROVIDERS)[number]

export interface CustomOpenClawProviderConfig {
  providerId: string
  apiKeyEnvVar: string
  config: Record<string, unknown>
}

export interface ResolvedOpenClawProviderConfig {
  envValues: Record<string, string>
  model?: string
  providerType?: SupportedOpenClawProvider
  customProvider?: CustomOpenClawProviderConfig
}

const PROVIDER_ENV_VARS: Record<SupportedOpenClawProvider, string> = {
  anthropic: 'ANTHROPIC_API_KEY',
  moonshot: 'MOONSHOT_API_KEY',
  openai: 'OPENAI_API_KEY',
  openrouter: 'OPENROUTER_API_KEY',
}

export class UnsupportedOpenClawProviderError extends Error {
  constructor(providerType: string) {
    super(`Unsupported OpenClaw provider: ${providerType}`)
    this.name = 'UnsupportedOpenClawProviderError'
  }
}

export function isUnsupportedOpenClawProviderError(
  error: unknown,
): error is UnsupportedOpenClawProviderError {
  return (
    error instanceof UnsupportedOpenClawProviderError ||
    (error instanceof Error &&
      error.name === 'UnsupportedOpenClawProviderError')
  )
}

export function isSupportedOpenClawProvider(
  providerType: string,
): providerType is SupportedOpenClawProvider {
  return SUPPORTED_OPENCLAW_PROVIDERS.includes(
    providerType as SupportedOpenClawProvider,
  )
}

export function assertSupportedOpenClawProvider(
  providerType?: string,
): SupportedOpenClawProvider | undefined {
  if (!providerType) {
    return undefined
  }
  if (!isSupportedOpenClawProvider(providerType)) {
    throw new UnsupportedOpenClawProviderError(providerType)
  }
  return providerType
}

export function buildOpenClawModelRef(
  providerType: SupportedOpenClawProvider,
  modelId?: string,
): string | undefined {
  return modelId ? `${providerType}/${modelId}` : undefined
}

export function deriveOpenClawProviderId(input: {
  providerType?: string
  providerName?: string
  baseUrl?: string
}): string {
  const source =
    input.providerName?.trim() ||
    input.baseUrl?.trim() ||
    input.providerType?.trim() ||
    'custom-provider'

  const candidate = source
    .toLowerCase()
    .replace(/^https?:\/\//, '')
    .replace(/[^a-z0-9]+/g, '-')
    .replace(/^-|-$/g, '')

  return candidate || 'custom-provider'
}

export function deriveOpenClawApiKeyEnvVar(providerId: string): string {
  return `${providerId.toUpperCase().replace(/-/g, '_')}_API_KEY`
}

export function getOpenClawProviderEnvVar(
  providerType: SupportedOpenClawProvider,
): string {
  return PROVIDER_ENV_VARS[providerType]
}

export function resolveSupportedOpenClawProvider(input: {
  providerType?: string
  providerName?: string
  baseUrl?: string
  apiKey?: string
  modelId?: string
}): ResolvedOpenClawProviderConfig {
  if (!input.providerType) {
    return { envValues: {} }
  }

  if (isSupportedOpenClawProvider(input.providerType)) {
    const providerType = input.providerType
    const envVar = getOpenClawProviderEnvVar(providerType)
    return {
      envValues: input.apiKey ? { [envVar]: input.apiKey } : {},
      model: buildOpenClawModelRef(providerType, input.modelId),
      providerType,
    }
  }

  if (!input.baseUrl) {
    throw new UnsupportedOpenClawProviderError(input.providerType)
  }

  const providerId = deriveOpenClawProviderId(input)
  const apiKeyEnvVar = deriveOpenClawApiKeyEnvVar(providerId)

  return {
    envValues: input.apiKey ? { [apiKeyEnvVar]: input.apiKey } : {},
    model: input.modelId ? `${providerId}/${input.modelId}` : undefined,
    customProvider: {
      providerId,
      apiKeyEnvVar,
      config: {
        api: 'openai-completions',
        baseUrl: input.baseUrl,
        apiKey: `\${${apiKeyEnvVar}}`,
        ...(input.modelId
          ? {
              models: [{ id: input.modelId, name: input.modelId }],
            }
          : {}),
      },
    },
  }
}
