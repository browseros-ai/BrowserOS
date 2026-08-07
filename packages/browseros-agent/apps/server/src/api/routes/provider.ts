/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { testProviderConnection } from '../../lib/clients/llm/test-provider'
import { logger } from '../../lib/logger'
import { AgentLLMConfigSchema } from '../types'

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
