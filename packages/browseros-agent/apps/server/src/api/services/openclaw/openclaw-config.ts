/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure functions for building OpenClaw bootstrap configuration.
 * Config is write-once at setup — agent CRUD uses WS RPC, not config edits.
 */

import {
  OPENCLAW_CONTAINER_HOME,
  OPENCLAW_GATEWAY_PORT,
} from '@browseros/shared/constants/openclaw'
import { DEFAULT_PORTS } from '@browseros/shared/constants/ports'

const OPENCLAW_IMAGE = 'ghcr.io/openclaw/openclaw:latest'

export const PROVIDER_ENV_MAP: Record<string, string> = {
  anthropic: 'ANTHROPIC_API_KEY',
  openai: 'OPENAI_API_KEY',
  google: 'GEMINI_API_KEY',
  openrouter: 'OPENROUTER_API_KEY',
  moonshot: 'MOONSHOT_API_KEY',
  groq: 'GROQ_API_KEY',
  mistral: 'MISTRAL_API_KEY',
}

export interface OpenClawProviderInput {
  providerType?: string
  providerName?: string
  baseUrl?: string
  modelId?: string
  apiKey?: string
}

export interface BootstrapConfigInput {
  gatewayPort: number
  gatewayToken: string
  browserosServerPort?: number
  providerType?: string
  providerName?: string
  baseUrl?: string
  modelId?: string
}

export interface EnvFileInput {
  image?: string
  port?: number
  token: string
  configDir: string
  timezone?: string
  providerKeys?: Record<string, string>
}

export interface ResolvedProviderConfig {
  model?: string
  providerKeys: Record<string, string>
  models?: {
    mode: 'merge'
    providers: Record<string, Record<string, unknown>>
  }
}

function hasBuiltinProvider(providerType?: string): providerType is string {
  return !!providerType && providerType in PROVIDER_ENV_MAP
}

export function deriveOpenClawProviderId(providerInput: {
  providerType?: string
  providerName?: string
  baseUrl?: string
}): string {
  const source =
    providerInput.providerName?.trim() ||
    providerInput.baseUrl?.trim() ||
    providerInput.providerType?.trim() ||
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

export function resolveProviderConfig(
  input: OpenClawProviderInput,
): ResolvedProviderConfig {
  if (!input.providerType) {
    return { providerKeys: {} }
  }

  if (hasBuiltinProvider(input.providerType)) {
    const providerKeys: Record<string, string> = {}
    if (input.apiKey) {
      providerKeys[PROVIDER_ENV_MAP[input.providerType]] = input.apiKey
    }

    return {
      providerKeys,
      model: input.modelId
        ? `${input.providerType}/${input.modelId}`
        : undefined,
    }
  }

  if (!input.baseUrl) {
    return { providerKeys: {} }
  }

  const providerId = deriveOpenClawProviderId(input)
  const apiKeyEnvVar = deriveOpenClawApiKeyEnvVar(providerId)
  const providerKeys: Record<string, string> = {}

  if (input.apiKey) {
    providerKeys[apiKeyEnvVar] = input.apiKey
  }

  const providerConfig: Record<string, unknown> = {
    baseUrl: input.baseUrl,
    apiKey: `\${${apiKeyEnvVar}}`,
    api: 'openai-completions',
  }

  if (input.modelId) {
    providerConfig.models = [{ id: input.modelId, name: input.modelId }]
  }

  return {
    providerKeys,
    model: input.modelId ? `${providerId}/${input.modelId}` : undefined,
    models: {
      mode: 'merge',
      providers: {
        [providerId]: providerConfig,
      },
    },
  }
}

export function buildBootstrapConfig(
  input: BootstrapConfigInput,
): Record<string, unknown> {
  const serverPort = input.browserosServerPort ?? DEFAULT_PORTS.server
  const provider = resolveProviderConfig(input)

  const defaults: Record<string, unknown> = {
    workspace: `${OPENCLAW_CONTAINER_HOME}/workspace`,
    timeoutSeconds: 4200,
    thinkingDefault: 'adaptive',
  }

  if (provider.model) {
    defaults.model = { primary: provider.model }
  }
  const config: Record<string, unknown> = {
    gateway: {
      mode: 'local',
      port: input.gatewayPort,
      bind: 'lan',
      auth: { mode: 'token', token: input.gatewayToken },
      reload: { mode: 'restart' },
      controlUi: {
        allowInsecureAuth: true,
        allowedOrigins: [
          `http://127.0.0.1:${input.gatewayPort}`,
          `http://localhost:${input.gatewayPort}`,
        ],
      },
      http: {
        endpoints: {
          chatCompletions: { enabled: true },
        },
      },
    },
    agents: { defaults },
    tools: {
      profile: 'full',
      web: {
        search: { provider: 'duckduckgo', enabled: true },
      },
      exec: {
        host: 'gateway',
        security: 'full',
        ask: 'off',
      },
    },
    cron: { enabled: true },
    hooks: {
      internal: {
        enabled: true,
        entries: {
          'boot-md': { enabled: true },
          'bootstrap-extra-files': { enabled: true },
          'session-memory': { enabled: true },
        },
      },
    },
    mcp: {
      servers: {
        browseros: {
          url: `http://host.containers.internal:${serverPort}/mcp`,
          transport: 'streamable-http',
        },
      },
    },
    approvals: {
      exec: { enabled: false },
    },
    skills: {
      install: { nodeManager: 'bun' },
    },
  }

  if (provider.models) {
    config.models = provider.models
  }

  return config
}

export function buildEnvFile(input: EnvFileInput): string {
  const lines: string[] = [
    `OPENCLAW_IMAGE=${input.image ?? OPENCLAW_IMAGE}`,
    `OPENCLAW_GATEWAY_PORT=${input.port ?? OPENCLAW_GATEWAY_PORT}`,
    `OPENCLAW_GATEWAY_TOKEN=${input.token}`,
    `OPENCLAW_CONFIG_DIR=${input.configDir}`,
    `TZ=${input.timezone ?? Intl.DateTimeFormat().resolvedOptions().timeZone}`,
  ]

  if (input.providerKeys) {
    for (const [key, value] of Object.entries(input.providerKeys)) {
      lines.push(`${key}=${value}`)
    }
  }

  return `${lines.join('\n')}\n`
}

export function resolveProviderKeys(
  input: OpenClawProviderInput,
): Record<string, string> {
  return resolveProviderConfig(input).providerKeys
}

export function resolveProviderModel(
  input: OpenClawProviderInput,
): string | undefined {
  return resolveProviderConfig(input).model
}
