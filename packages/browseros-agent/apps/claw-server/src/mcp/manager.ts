/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Per-slug MCP server orchestration. Resolves an incoming `/mcp/:slug`
 * request against the agents directory, builds a fresh `McpServer`
 * with the catalog of tools registered for that agent, and lets the
 * SDK's Web Standard transport handle the actual HTTP framing.
 *
 * Why per-request servers instead of cached singletons:
 *   The SDK's `Protocol.connect(transport)` rejects when the server
 *   is already connected to a different transport, and stateless
 *   Streamable HTTP transports are single-use by design. The cleanest
 *   way to support concurrent requests for the same slug is to build
 *   a fresh server + transport per request. The cost is a handful of
 *   `registerTool` calls (each just sets a Map entry), which is
 *   negligible.
 */

import { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { WebStandardStreamableHTTPServerTransport } from '@modelcontextprotocol/sdk/server/webStandardStreamableHttp.js'
import { logger } from '../lib/logger'
import { findBySlug } from '../routes/agents/service'
import { registerBrowserTools } from './register'

const SERVER_NAME = 'browseros-claw-server'
const SERVER_VERSION = '0.0.1'

/**
 * Handles a single `/mcp/:slug` request end-to-end. Returns either:
 *   - 404 Response if the slug doesn't match any agent profile, OR
 *   - the SDK's Response from the Streamable HTTP transport.
 */
export async function handleMcpRequest(
  slug: string,
  request: Request,
): Promise<Response> {
  const agent = await findBySlug(slug)
  if (!agent) {
    return new Response(JSON.stringify({ error: 'agent not found' }), {
      status: 404,
      headers: { 'content-type': 'application/json' },
    })
  }

  const server = new McpServer({
    name: SERVER_NAME,
    title: `BrowserOS / ${agent.name}`,
    version: SERVER_VERSION,
  })

  registerBrowserTools(server, agent)

  // Stateless mode: each request gets its own short-lived transport,
  // we return its Response directly. JSON response is enabled so the
  // simplest clients (curl, MCP inspector) work without negotiating
  // SSE framing.
  const transport = new WebStandardStreamableHTTPServerTransport({
    sessionIdGenerator: undefined,
    enableJsonResponse: true,
  })

  try {
    await server.connect(transport)
    return await transport.handleRequest(request)
  } catch (err) {
    logger.error('mcp request failed', {
      slug,
      error: err instanceof Error ? err.message : String(err),
    })
    return new Response(JSON.stringify({ error: 'internal mcp error' }), {
      status: 500,
      headers: { 'content-type': 'application/json' },
    })
  }
}
