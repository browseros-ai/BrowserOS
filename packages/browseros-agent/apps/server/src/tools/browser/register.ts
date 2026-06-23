import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { ZodRawShape } from 'zod'
import type { BrowserSession } from '../../browser/core/session'
import { logger } from '../../lib/logger'
import { metrics } from '../../lib/metrics'
import { shouldLogToolRegistration } from '../registration-log-sampling'
import { executeTool } from './framework'
import {
  type BrowserOutputFileAccess,
  withBrowserOutputFileAccess,
} from './output-file'
import { BROWSER_TOOLS } from './registry'

// The SDK's registerTool is heavily overloaded/generic; retyping it to a concrete signature
// avoids a TS "excessively deep" instantiation while keeping the call shape honest.
type RegisterFn = (
  name: string,
  config: {
    description: string
    inputSchema?: ZodRawShape
    annotations?: Record<string, unknown>
  },
  handler: (
    args: Record<string, unknown>,
    extra?: { signal?: AbortSignal },
  ) => Promise<{
    content: unknown
    isError?: boolean
    structuredContent?: unknown
  }>,
) => void

export interface BrowserToolDefaults {
  defaultWindowId?: number
  defaultTabGroupId?: string
}

export interface BrowserToolRegistrationOptions {
  outputFileAccess?: BrowserOutputFileAccess
}

/**
 * Registers the browser-core tool surface on an MCP server, all bound to one BrowserSession.
 */
export function registerBrowserTools(
  server: McpServer,
  session: BrowserSession,
  defaults: BrowserToolDefaults = {},
  options: BrowserToolRegistrationOptions = {},
): void {
  const register = server.registerTool.bind(server) as unknown as RegisterFn

  for (const tool of BROWSER_TOOLS) {
    register(
      tool.name,
      {
        description: tool.description,
        inputSchema: tool.input.shape,
        ...(tool.annotations && {
          annotations: tool.annotations as Record<string, unknown>,
        }),
      },
      async (args, extra) => {
        const startTime = performance.now()
        try {
          const result = await withBrowserOutputFileAccess(
            options.outputFileAccess,
            () =>
              executeTool(tool, args, {
                session,
                ...defaults,
                signal: extra?.signal,
              }),
          )
          metrics.log('tool_executed', {
            tool_name: tool.name,
            duration_ms: Math.round(performance.now() - startTime),
            success: !result.isError,
            source: 'mcp',
          })
          return {
            content: result.content,
            isError: result.isError,
            structuredContent: result.structuredContent,
          }
        } catch (error) {
          const errorText =
            error instanceof Error ? error.message : String(error)
          metrics.log('tool_executed', {
            tool_name: tool.name,
            duration_ms: Math.round(performance.now() - startTime),
            success: false,
            error_message: errorText,
            source: 'mcp',
          })
          return {
            content: [{ type: 'text' as const, text: errorText }],
            isError: true,
          }
        }
      },
    )
  }

  if (shouldLogToolRegistration()) {
    logger.info(
      `Registered ${BROWSER_TOOLS.length} browser tools: ${BROWSER_TOOLS.map((t) => t.name).join(', ')}`,
    )
  }
}
