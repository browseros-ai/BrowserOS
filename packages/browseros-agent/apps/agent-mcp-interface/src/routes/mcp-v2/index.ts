/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 single MCP endpoint. Every agent connects to the same standard
 * URL; identity is captured at handshake via the shared transport's
 * `onsessioninitialized` hook in `single-server.ts`. This route is
 * always mounted; turning off the legacy per-slug route does not
 * affect this one. The host server mounts the cockpit at `/cockpit`,
 * so the resulting public URL is `POST /cockpit/mcp`.
 */

import { Hono } from 'hono'
import { getSingleMcpInstance } from '../../mcp/single-server'

export const mcpV2Route = new Hono().all('/mcp', async (c) => {
  const { transport } = getSingleMcpInstance()
  return transport.handleRequest(c.req.raw)
})
