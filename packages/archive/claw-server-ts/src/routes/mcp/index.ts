/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Single MCP endpoint. Every agent connects to the same standard URL;
 * identity is captured via the server's `oninitialized` hook in
 * `single-server.ts` (which fires on the InitializedNotification,
 * after the server has stored `clientInfo`). The standalone server
 * serves the route at `/mcp`.
 */

import { Hono } from 'hono'
import { handleSingleMcpRequest } from '../../mcp/single-server'
import { setMcpRequestHygieneMiddleware } from './mcp-request-hygiene'

export const mcpRoute = new Hono()
  .use('/mcp', setMcpRequestHygieneMiddleware)
  .all('/mcp', async (c) => {
    return handleSingleMcpRequest(c.req.raw)
  })
