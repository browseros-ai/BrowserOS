/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { Hono } from 'hono'
import {
  humaniseInstallError,
  installInto,
  listAgents,
  uninstallFrom,
} from '../../lib/mcp-manager'

interface McpManagerRouteOptions {
  /**
   * Returns the BrowserOS MCP URL the running server bound to. Hot
   * because the URL can change between server restarts, so the route
   * reads it per-request rather than caching at module load time.
   */
  getMcpUrl: () => string
}

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
      try {
        const result = await installInto(id, getMcpUrl())
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
