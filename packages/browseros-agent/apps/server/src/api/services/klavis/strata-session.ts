/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { TIMEOUTS } from '@browseros/shared/constants/timeouts'
import { Client } from '@modelcontextprotocol/sdk/client/index.js'
import { StreamableHTTPClientTransport } from '@modelcontextprotocol/sdk/client/streamableHttp.js'
import type { CallToolResult } from '@modelcontextprotocol/sdk/types.js'
import { jsonSchemaObjectToZodRawShape } from 'zod-from-json-schema'
import type { KlavisClient } from './client'
import type { KlavisStrataCache } from './strata-cache'
import type { KlavisSessionHandle } from './types'

function withTimeout<T>(promise: Promise<T>, label: string): Promise<T> {
  let timerId: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<T>((_, reject) => {
    timerId = setTimeout(
      () => reject(new Error(`Klavis ${label} timed out`)),
      TIMEOUTS.KLAVIS_FETCH,
    )
  })
  return Promise.race([promise, timeout]).finally(() => clearTimeout(timerId))
}

export interface ConnectKlavisStrataSessionDeps {
  client: KlavisClient
  cache: KlavisStrataCache
  browserosId: string
  servers: readonly string[]
}

/** Opens one MCP client session for the BrowserOS-managed Klavis Strata server. */
export async function connectKlavisStrataSession(
  deps: ConnectKlavisStrataSessionDeps,
): Promise<KlavisSessionHandle> {
  const strata = await deps.cache.getOrFetch(
    deps.client,
    deps.browserosId,
    deps.servers,
  )

  const client = new Client({
    name: 'browseros-klavis-proxy',
    version: '1.0.0',
  })
  const transport = new StreamableHTTPClientTransport(
    new URL(strata.strataServerUrl),
  )
  await withTimeout(client.connect(transport), 'connect')

  const { tools } = await withTimeout(client.listTools(), 'listTools')

  const inputSchemas = new Map(
    tools.map((t) => [
      t.name,
      jsonSchemaObjectToZodRawShape(
        t.inputSchema as never,
      ) as unknown as Record<string, never>,
    ]),
  )

  return {
    browserosId: deps.browserosId,
    tools,
    inputSchemas,
    callTool: (name, args) =>
      withTimeout(
        client.callTool({ name, arguments: args }) as Promise<CallToolResult>,
        `callTool(${name})`,
      ),
    close: () => client.close(),
  }
}
