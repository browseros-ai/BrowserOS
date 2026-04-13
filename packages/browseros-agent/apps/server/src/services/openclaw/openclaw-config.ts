/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Pure functions for building OpenClaw bootstrap configuration.
 * Config is write-once at setup — agent CRUD uses WS RPC, not config edits.
 */

import { DEFAULT_PORTS } from '@browseros/shared/constants/ports'

const OPENCLAW_IMAGE = 'ghcr.io/openclaw/openclaw:latest'
const OPENCLAW_GATEWAY_PORT = 18789
const CONTAINER_HOME = '/home/node/.openclaw'

export const PROVIDER_ENV_MAP: Record<string, string> = {
  anthropic: 'ANTHROPIC_API_KEY',
  openai: 'OPENAI_API_KEY',
  google: 'GEMINI_API_KEY',
  openrouter: 'OPENROUTER_API_KEY',
  moonshot: 'MOONSHOT_API_KEY',
  groq: 'GROQ_API_KEY',
  mistral: 'MISTRAL_API_KEY',
}

export interface BootstrapConfigInput {
  gatewayPort: number
  gatewayToken: string
  browserosServerPort?: number
  providerType?: string
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

export function buildBootstrapConfig(
  input: BootstrapConfigInput,
): Record<string, unknown> {
  const serverPort = input.browserosServerPort ?? DEFAULT_PORTS.server

  const defaults: Record<string, unknown> = {
    workspace: `${CONTAINER_HOME}/workspace`,
    timeoutSeconds: 4200,
    thinkingDefault: 'adaptive',
  }

  if (input.providerType && input.modelId) {
    defaults.model = { primary: `${input.providerType}/${input.modelId}` }
  }

  return {
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
  providerType?: string,
  apiKey?: string,
): Record<string, string> {
  const keys: Record<string, string> = {}
  if (!providerType || !apiKey) return keys

  const envVar = PROVIDER_ENV_MAP[providerType]
  if (envVar) {
    keys[envVar] = apiKey
  }
  return keys
}
