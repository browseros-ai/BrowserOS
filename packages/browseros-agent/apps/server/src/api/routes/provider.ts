/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { testProviderConnection } from '../../lib/clients/llm/test-provider'
import { logger } from '../../lib/logger'
import {
  dbProviderStore,
  type ProviderStore,
} from '../../lib/providers/provider-store'
import { AgentLLMConfigSchema } from '../types'

interface ProviderRouteDeps {
  browserosId?: string
  store?: Pick<ProviderStore, 'getWithCredentials'>
}

const CREDENTIAL_FIELDS = [
  'apiKey',
  'accessKeyId',
  'secretAccessKey',
  'sessionToken',
] as const

type StoredCredentials = {
  type: string
} & Partial<Record<(typeof CREDENTIAL_FIELDS)[number], string | null>>

/**
 * Fills blank credential fields from the saved provider row so testing an
 * existing provider works even though reads redact its secrets. Only reuses a
 * stored secret when the provider type is unchanged, so a form switched to a
 * different type must supply that type's own credential rather than silently
 * reusing the previous one.
 */
export function mergeStoredCredentials<T extends { provider: string }>(
  config: T,
  stored: StoredCredentials | null,
): T {
  if (!stored || stored.type !== config.provider) return config
  const merged = { ...config } as Record<string, unknown>
  for (const field of CREDENTIAL_FIELDS) {
    if (!merged[field]) merged[field] = stored[field] ?? undefined
  }
  return merged as T
}

export function createProviderRoutes(deps: ProviderRouteDeps = {}) {
  const store = deps.store ?? dbProviderStore
  return new Hono().post(
    '/',
    zValidator('json', AgentLLMConfigSchema),
    async (c) => {
      const config = c.req.valid('json')
      const stored = config.providerId
        ? await store.getWithCredentials(config.providerId)
        : null
      const resolved = mergeStoredCredentials(config, stored)

      logger.info('Testing provider connection', {
        provider: resolved.provider,
        model: resolved.model,
      })

      const result = await testProviderConnection(resolved, deps.browserosId)

      logger.info('Provider test result', {
        provider: resolved.provider,
        model: resolved.model,
        success: result.success,
        responseTime: result.responseTime,
      })

      return c.json(result, result.success ? 200 : 400)
    },
  )
}
