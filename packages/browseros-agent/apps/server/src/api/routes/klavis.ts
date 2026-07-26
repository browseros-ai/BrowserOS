/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { zValidator } from '@hono/zod-validator'
import { Hono } from 'hono'
import { z } from 'zod'
import { logger } from '../../lib/logger'
import { isSupportedConnector, type KlavisService } from '../services/klavis'

const ServerNameSchema = z.object({
  serverName: z.string().min(1),
})

interface KlavisRouteDeps {
  klavis: KlavisService
}

export function createKlavisRoutes(deps: KlavisRouteDeps) {
  const { klavis } = deps

  return new Hono()
    .get('/servers', (c) => {
      const servers = klavis.listAvailableConnectors()
      return c.json({
        servers,
        count: servers.length,
      })
    })
    .get('/oauth-urls', async (c) => {
      try {
        const serverNames = klavis.listAvailableConnectors().map((s) => s.name)
        const intents = await klavis.createConnectionIntents(serverNames)

        logger.info('Generated OAuth URLs', {
          serverCount: serverNames.length,
        })

        return c.json({
          oauthUrls: Object.fromEntries(
            intents
              .filter((intent) => intent.oauthUrl)
              .map((intent) => [intent.serverName, intent.oauthUrl]),
          ),
          servers: serverNames,
        })
      } catch (error) {
        if (
          error instanceof Error &&
          error.message === 'browserosId not configured'
        ) {
          return c.json({ error: 'browserosId not configured' }, 500)
        }
        logger.error('Error getting OAuth URLs', {
          error: error instanceof Error ? error.message : String(error),
        })
        return c.json({ error: 'Failed to get OAuth URLs' }, 500)
      }
    })
    .get('/user-integrations', async (c) => {
      try {
        const integrations = await klavis.getUserIntegrations()
        const normalizedIntegrations = integrations.map((integration) => ({
          name: integration.name,
          is_authenticated: integration.isAuthenticated,
        }))
        logger.info('Fetched user integrations', {
          count: normalizedIntegrations.length,
        })
        return c.json({
          integrations: normalizedIntegrations,
          count: normalizedIntegrations.length,
        })
      } catch (error) {
        if (
          error instanceof Error &&
          error.message === 'browserosId not configured'
        ) {
          return c.json({ error: 'browserosId not configured' }, 500)
        }
        logger.error('Error fetching user integrations', {
          error: error instanceof Error ? error.message : String(error),
        })
        return c.json({ error: 'Failed to fetch user integrations' }, 500)
      }
    })
    .post('/servers/add', zValidator('json', ServerNameSchema), async (c) => {
      const { serverName } = c.req.valid('json')

      if (!isSupportedConnector(serverName)) {
        return c.json({ error: `Invalid server: ${serverName}` }, 400)
      }

      logger.info('Adding server to strata', { serverName })

      try {
        const result = await klavis.createConnectionIntent(serverName)

        return c.json({
          success: true,
          serverName,
          strataId: result.strataId,
          addedServers: result.addedServers,
          oauthUrl: result.oauthUrl,
          apiKeyUrl: result.apiKeyUrl,
        })
      } catch (error) {
        if (
          error instanceof Error &&
          error.message === 'browserosId not configured'
        ) {
          return c.json({ error: 'browserosId not configured' }, 500)
        }
        logger.error('Error adding server to strata', {
          serverName,
          error: error instanceof Error ? error.message : String(error),
        })
        return c.json({ error: 'Failed to add server' }, 500)
      }
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
        const { serverName, apiKey, apiKeyUrl } = c.req.valid('json')

        if (!isSupportedConnector(serverName)) {
          return c.json({ error: `Invalid server: ${serverName}` }, 400)
        }

        try {
          await klavis.submitApiKey({ serverName, apiKey, apiKeyUrl })

          logger.info('Submitted API key for server', { serverName })

          return c.json({ success: true, serverName })
        } catch (error) {
          if (
            error instanceof Error &&
            error.message === 'browserosId not configured'
          ) {
            return c.json({ error: 'browserosId not configured' }, 500)
          }
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
        const { serverName } = c.req.valid('json')

        if (!isSupportedConnector(serverName)) {
          return c.json({ error: `Invalid server: ${serverName}` }, 400)
        }

        logger.info('Removing server from strata', { serverName })

        try {
          await klavis.removeConnector(serverName)

          return c.json({
            success: true,
            serverName,
          })
        } catch (error) {
          if (
            error instanceof Error &&
            error.message === 'browserosId not configured'
          ) {
            return c.json({ error: 'browserosId not configured' }, 500)
          }
          logger.error('Error removing server from strata', {
            serverName,
            error: error instanceof Error ? error.message : String(error),
          })
          return c.json({ error: 'Failed to remove server' }, 500)
        }
      },
    )
}
