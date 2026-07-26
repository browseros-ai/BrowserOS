/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Detects MCP transport type by probing the server endpoint.
 *
 * Following MCP spec (2025-03-26): try Streamable HTTP first, fall back to SSE.
 * SSE transport is deprecated but still supported for older servers.
 *
 * @see https://modelcontextprotocol.io/specification/2025-03-26/basic/transports
 */

import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { logger } from './logger'

export type McpTransportType = 'streamable-http' | 'sse'

const CACHE_TTL_MS = 60 * 60 * 1000 // 1 hour

interface CachedTransport {
  transport: McpTransportType
  expiresAt: number
}

const transportCache = new Map<string, CachedTransport>()

export async function detectMcpTransport(
  url: string,
): Promise<McpTransportType> {
  const cached = transportCache.get(url)
  if (cached && Date.now() < cached.expiresAt) {
    return cached.transport
  }

  let transport: McpTransportType
  let shouldCache = true
  try {
    const response = await fetch(url, {
      method: 'POST',
      headers: {
        'Content-Type': 'application/json',
        Accept: 'application/json, text/event-stream',
      },
      body: JSON.stringify({
        jsonrpc: '2.0',
        id: 0,
        method: 'initialize',
        params: {
          protocolVersion: '2025-03-26',
          capabilities: {},
          clientInfo: { name: 'browseros-probe', version: '1.0.0' },
        },
      }),
      signal: AbortSignal.timeout(TIMEOUTS.MCP_TRANSPORT_PROBE),
    })

    const contentType = response.headers.get('content-type') ?? ''
    const isJsonResponse = contentType.includes('application/json')
    const isStreamResponse = contentType.includes('text/event-stream')

    // Release the connection — avoid leaking open streams (especially SSE)
    await response.body?.cancel()

    logger.debug('MCP transport probe response', {
      url,
      status: response.status,
      contentType,
    })

    if (response.status === 200 && (isJsonResponse || isStreamResponse)) {
      transport = 'streamable-http'
    } else if (response.status === 401) {
      transport = 'streamable-http'
    } else if (response.status >= 500) {
      transport = 'sse'
      shouldCache = false
    } else {
      transport = 'sse'
    }
  } catch (error) {
    logger.debug('MCP transport probe failed', {
      url,
      error: error instanceof Error ? error.message : String(error),
    })
    transport = 'sse'
    shouldCache = false
  }

  if (shouldCache) {
    transportCache.set(url, {
      transport,
      expiresAt: Date.now() + CACHE_TTL_MS,
    })
  }
  logger.debug('Detected MCP transport', {
    url,
    transport,
    cached: shouldCache,
  })
  return transport
}
