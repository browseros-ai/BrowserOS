/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * `/mcp/:slug` is the HTTP entry point a harness (Claude Desktop,
 * Cursor, Codex, Zed) hits to talk to the cockpit. The slug is the
 * route segment the wizard generated for an agent; everything past
 * the route layer is delegated to the per-slug MCP manager which
 * owns the SDK plumbing.
 *
 * `app.all` matches every method the Streamable HTTP transport
 * understands (POST for JSON-RPC, GET for SSE, DELETE for session
 * termination). The transport handles method dispatch internally.
 */

import { Hono } from 'hono'
import { isCockpitLegacyPerAgentMcpEnabled } from '../../env'
import { handleMcpRequest } from '../../mcp/manager'

export const mcpRoute = new Hono().all('/mcp/:slug', async (c) => {
  if (!isCockpitLegacyPerAgentMcpEnabled()) {
    return c.json(
      {
        error: 'legacy per-agent MCP endpoint is disabled',
        hint: 'use the standard endpoint at /cockpit/mcp, or set COCKPIT_LEGACY_PER_AGENT_MCP=1 to re-enable per-slug routes',
      },
      404,
    )
  }
  return handleMcpRequest(c.req.param('slug'), c.req.raw)
})
