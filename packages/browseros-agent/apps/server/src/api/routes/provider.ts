/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { isAcpProvider } from '../../agent/acp-providers'
import { testAcpProvider } from '../../lib/clients/llm/test-acp-provider'
import { testProviderConnection } from '../../lib/clients/llm/test-provider'
import { logger } from '../../lib/logger'
import { AgentLLMConfigSchema } from '../types'

interface ProviderRouteDeps {
  browserosId?: string
  resourcesDir?: string | null
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

      const result = isAcpProvider(config.provider)
        ? await testAcpProvider(
            {
              provider: config.provider,
              model: config.model,
              acpAgentId: config.acpAgentId,
              acpCommand: config.acpCommand,
              acpFixedWorkspacePath: config.acpFixedWorkspacePath,
            },
            { resourcesDir: deps.resourcesDir },
          )
        : await testProviderConnection(config, deps.browserosId)

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
