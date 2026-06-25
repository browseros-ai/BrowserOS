/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * v2 single MCP endpoint. Every agent connects to the same standard
 * URL; identity is captured via the server's `oninitialized` hook in
 * `single-server.ts` (which fires on the InitializedNotification,
 * after the server has stored `clientInfo`). This route is always
 * mounted; turning off the legacy per-slug route does not affect
 * this one. The standalone server mounts the cockpit at `/cockpit`,
 * so the resulting public URL is `POST /cockpit/mcp`.
 */

import { Hono } from 'hono'
import { handleSingleMcpRequest } from '../../mcp/single-server'

export const mcpV2Route = new Hono().all('/mcp', async (c) => {
  return handleSingleMcpRequest(c.req.raw)
})
