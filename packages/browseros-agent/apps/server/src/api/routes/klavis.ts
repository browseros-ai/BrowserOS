/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { KlavisClient } from '../../lib/clients/klavis/klavis-client'
import { OAUTH_MCP_SERVERS } from '../../lib/clients/klavis/oauth-mcp-servers'
import { logger } from '../../lib/logger'
import { klavisStrataCache } from '../services/klavis/strata-cache'

const ServerNameSchema = z.object({
  serverName: z.string().min(1),
})

interface KlavisRouteDeps {
  browserosId: string
}

const normalizeServerKey = (value: string): string =>
  value.toLowerCase().replace(/[^a-z0-9]+/g, '')

const getAuthUrlForServer = (
  authUrlMap: Record<string, string> | undefined,
  serverName: string,
): string | undefined => {
  if (!authUrlMap) {
    return undefined
  }
  const directMatch = authUrlMap[serverName]
  if (directMatch) {
    return directMatch
  }
  const targetKey = normalizeServerKey(serverName)
  for (const [key, value] of Object.entries(authUrlMap)) {
    if (normalizeServerKey(key) === targetKey) {
      return value
    }
  }
  return undefined
}

export function createKlavisRoutes(deps: KlavisRouteDeps) {
  const { browserosId } = deps
  const klavisClient = new KlavisClient()

  // Chain route definitions for proper Hono RPC type inference
  return new Hono()
    .get('/servers', (c) => {
      return c.json({
        servers: OAUTH_MCP_SERVERS,
        count: OAUTH_MCP_SERVERS.length,
      })
    })
    .get('/oauth-urls', async (c) => {
      if (!browserosId) {
        return c.json({ error: 'browserosId not configured' }, 500)
      }

      try {
        const serverNames = OAUTH_MCP_SERVERS.map((s) => s.name)
        const response = await klavisClient.createStrata(
          browserosId,
          serverNames,
        )

        logger.info('Generated OAuth URLs', {
          browserosId: browserosId.slice(0, 12),
          serverCount: serverNames.length,
        })

        return c.json({
          oauthUrls: response.oauthUrls || {},
          servers: serverNames,
        })
      } catch (error) {
        logger.error('Error getting OAuth URLs', {
          browserosId: browserosId?.slice(0, 12),
          error: error instanceof Error ? error.message : String(error),
        })
        return c.json({ error: 'Failed to get OAuth URLs' }, 500)
      }
    })
    .get('/user-integrations', async (c) => {
      if (!browserosId) {
        return c.json({ error: 'browserosId not configured' }, 500)
      }

      try {
        const integrations = await klavisClient.getUserIntegrations(browserosId)
        const normalizedIntegrations = integrations.map((integration) => ({
          name: integration.name,
          is_authenticated: integration.isAuthenticated,
        }))
        logger.info('Fetched user integrations', {
          browserosId: browserosId.slice(0, 12),
          count: normalizedIntegrations.length,
        })
        return c.json({
          integrations: normalizedIntegrations,
          count: normalizedIntegrations.length,
        })
      } catch (error) {
        logger.error('Error fetching user integrations', {
          browserosId: browserosId?.slice(0, 12),
          error: error instanceof Error ? error.message : String(error),
        })
        return c.json({ error: 'Failed to fetch user integrations' }, 500)
      }
    })
    .post('/servers/add', zValidator('json', ServerNameSchema), async (c) => {
      if (!browserosId) {
        return c.json({ error: 'browserosId not configured' }, 500)
      }

      const { serverName } = c.req.valid('json')

      const validServer = OAUTH_MCP_SERVERS.find((s) => s.name === serverName)
      if (!validServer) {
        return c.json({ error: `Invalid server: ${serverName}` }, 400)
      }

      logger.info('Adding server to strata', { serverName })

      const result = await klavisClient.createStrata(browserosId, [serverName])
      klavisStrataCache.invalidate(browserosId)

      return c.json({
        success: true,
        serverName,
        strataId: result.strataId,
        addedServers: result.addedServers,
        oauthUrl: getAuthUrlForServer(result.oauthUrls, serverName),
        apiKeyUrl: getAuthUrlForServer(result.apiKeyUrls, serverName),
      })
    })
    .post(
      '/servers/submit-api-key',
      zValidator(
        'json',
        z.object({
          serverName: z.string().min(1),
          apiKey: z.string().min(1),
          apiKeyUrl: z.string().url(),
        }),
      ),
      async (c) => {
        if (!browserosId) {
          return c.json({ error: 'browserosId not configured' }, 500)
        }

        const { serverName, apiKey, apiKeyUrl } = c.req.valid('json')

        try {
          await klavisClient.submitApiKey(apiKeyUrl, apiKey)

          logger.info('Submitted API key for server', { serverName })

          return c.json({ success: true, serverName })
        } catch (error) {
          logger.error('Error submitting API key', {
            serverName,
            error: error instanceof Error ? error.message : String(error),
          })
          return c.json({ error: 'Failed to submit API key' }, 500)
        }
      },
    )
    .delete(
      '/servers/remove',
      zValidator('json', ServerNameSchema),
      async (c) => {
        if (!browserosId) {
          return c.json({ error: 'browserosId not configured' }, 500)
        }

        const { serverName } = c.req.valid('json')

        const validServer = OAUTH_MCP_SERVERS.find((s) => s.name === serverName)
        if (!validServer) {
          return c.json({ error: `Invalid server: ${serverName}` }, 400)
        }

        logger.info('Removing server from strata', { serverName })

        // The chat hot path keys its cache by the user's full enabled set,
        // so a single-server lookup here would always miss and immediately
        // be cleared by invalidate() below — call createStrata directly
        // to recover the strataId, mirroring the original removeServer flow.
        const strata = await klavisClient.createStrata(browserosId, [
          serverName,
        ])
        await klavisClient.deleteServersFromStrata(strata.strataId, [
          serverName,
        ])
        klavisStrataCache.invalidate(browserosId)

        return c.json({
          success: true,
          serverName,
        })
      },
    )
}
