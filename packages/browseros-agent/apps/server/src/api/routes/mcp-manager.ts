/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'
import { z } from 'zod'
import {
  humaniseInstallError,
  installInto,
  listAgents,
  uninstallFrom,
} from '../../lib/mcp-manager'

interface McpManagerRouteOptions {
  /**
   * Fallback BrowserOS MCP URL the agent server bound to internally.
   * Only used when the client doesn't pass `mcpUrl` in the install
   * request body. Hot because the URL can change between server
   * restarts.
   *
   * NOTE: the agent server's own port is NOT the URL external MCP
   * clients reach. The browser proxies `/mcp` from the user-facing
   * port to the agent server. The UI always sends the proxy URL via
   * `mcpUrl`; this fallback exists only for programmatic callers
   * that have no other source.
   */
  getMcpUrl: () => string
}

const InstallBodySchema = z
  .object({
    mcpUrl: z.string().url().optional(),
  })
  .partial()

export function createMcpManagerRoutes(options: McpManagerRouteOptions) {
  const { getMcpUrl } = options

  return new Hono()
    .get('/agents', async (c) => {
      try {
        const agents = await listAgents()
        return c.json({ agents })
      } catch (err) {
        const message = err instanceof Error ? err.message : String(err)
        return c.json({ message }, 500)
      }
    })
    .post('/agents/:id/install', async (c) => {
      const id = c.req.param('id')
      const raw = await c.req.json().catch(() => ({}))
      const parsed = InstallBodySchema.safeParse(raw)
      if (!parsed.success) {
        return c.json(
          {
            success: false,
            message: 'Invalid request body: mcpUrl must be a URL.',
          },
          400,
        )
      }
      const url = parsed.data.mcpUrl ?? getMcpUrl()
      try {
        const result = await installInto(id, url)
        return c.json(result, 200)
      } catch (err) {
        const { message, status } = humaniseInstallError(err)
        return c.json(
          { success: false, message },
          status as 400 | 404 | 409 | 500,
        )
      }
    })
    .post('/agents/:id/uninstall', async (c) => {
      const id = c.req.param('id')
      try {
        const result = await uninstallFrom(id)
        return c.json(result, result.success ? 200 : 409)
      } catch (err) {
        const { message, status } = humaniseInstallError(err)
        return c.json(
          { success: false, message },
          status as 400 | 404 | 409 | 500,
        )
      }
    })
}
