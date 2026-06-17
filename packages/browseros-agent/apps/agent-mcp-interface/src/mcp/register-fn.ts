/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * The SDK's `McpServer.registerTool` overload set is parameterised on
 * the SDK's internal zod-shape type, which mismatches with the zod v4
 * shape this package uses. Retyping `registerTool` to a concrete,
 * non-generic signature avoids a TS "excessively deep instantiation"
 * error while keeping the call shape honest.
 *
 * Same workaround `apps/server/src/tools/browser/register.ts` uses.
 */

import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { ZodRawShape } from 'zod'

export type ToolResultContent = {
  type: 'text'
  text: string
}

export interface ToolResult {
  content: ToolResultContent[]
  isError?: boolean
  structuredContent?: unknown
}

export type ToolHandler = (
  args: Record<string, unknown>,
  extra?: { signal?: AbortSignal },
) => Promise<ToolResult>

export type RegisterFn = (
  name: string,
  config: {
    description: string
    inputSchema?: ZodRawShape
    annotations?: Record<string, unknown>
  },
  handler: ToolHandler,
) => void

export function asRegister(server: McpServer): RegisterFn {
  return server.registerTool.bind(server) as unknown as RegisterFn
}
