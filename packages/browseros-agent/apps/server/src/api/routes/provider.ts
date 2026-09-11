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

type StoredProvider = {
  type: string
  baseUrl?: string | null
  resourceName?: string | null
  region?: string | null
} & Partial<Record<(typeof CREDENTIAL_FIELDS)[number], string | null>>

/**
 * Fills blank credential fields from the saved provider row so testing an
 * existing provider works even though reads redact its secrets.
 *
 * A reused stored secret is bound to the stored destination: whenever a blank
 * credential is filled in, the connection settings (base URL / resource name /
 * region) are taken from the saved row too, so the secret can only ever be sent
 * to the endpoint it was saved against, never a caller-supplied URL. A caller
 * that supplies its own credential keeps its own settings, since no stored
 * secret is at risk. The type must still match, so an openrouter key is never
 * reused as, say, an azure provider.
 */
export function mergeStoredCredentials<
  T extends {
    provider: string
    baseUrl?: string
    resourceName?: string
    region?: string
  },
>(config: T, stored: StoredProvider | null): T {
  if (!stored || stored.type !== config.provider) return config
  const merged = { ...config } as Record<string, unknown>
  let reusedStoredSecret = false
  for (const field of CREDENTIAL_FIELDS) {
    if (!merged[field] && stored[field]) {
      merged[field] = stored[field]
      reusedStoredSecret = true
    }
  }
  if (reusedStoredSecret) {
    merged.baseUrl = stored.baseUrl ?? undefined
    merged.resourceName = stored.resourceName ?? undefined
    merged.region = stored.region ?? undefined
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
