/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { listProviderModels } from '../../lib/clients/llm/list-models'
import { testProviderConnection } from '../../lib/clients/llm/test-provider'
import { logger } from '../../lib/logger'
import { AgentLLMConfigSchema } from '../types'

const ListModelsRequestSchema = z.object({
  provider: z.string().min(1),
  baseUrl: z.string().min(1),
  apiKey: z.string().optional(),
})

interface ProviderRouteDeps {
  browserosId?: string
}

export function createProviderRoutes(deps: ProviderRouteDeps = {}) {
  return new Hono().post(
    '/',
    zValidator('json', AgentLLMConfigSchema),
    async (c) => {
      const config = c.req.valid('json')

      logger.info('Testing provider connection', {
        provider: config.provider,
        model: config.model,
      })

      const result = await testProviderConnection(config, deps.browserosId)

      logger.info('Provider test result', {
        provider: config.provider,
        model: config.model,
        success: result.success,
        responseTime: result.responseTime,
      })

      return c.json(result, result.success ? 200 : 400)
    },
  )
}

/** Lists the models an OpenAI-compatible endpoint actually serves. */
export function createListModelRoutes() {
  return new Hono().post(
    '/',
    zValidator('json', ListModelsRequestSchema),
    async (c) => {
      const { provider, baseUrl, apiKey } = c.req.valid('json')

      logger.info('Listing provider models', { provider, baseUrl })

      try {
        const models = await listProviderModels({ provider, baseUrl, apiKey })
        logger.info('Provider models result', {
          provider,
          count: models.length,
        })
        return c.json({ models })
      } catch (error) {
        // Soft-fail so the client keeps its free-form model entry UX.
        const message = error instanceof Error ? error.message : String(error)
        logger.warn('Provider models lookup failed', { provider, message })
        return c.json({ models: [], message })
      }
    },
  )
}
