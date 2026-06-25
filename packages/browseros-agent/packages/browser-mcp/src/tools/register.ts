import type { BrowserSession } from '@browseros/browser-core/core/session'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { ZodRawShape } from 'zod'
import { executeTool } from './framework'
import {
  type BrowserOutputFileAccess,
  withBrowserOutputFileAccess,
} from './output-file'
import { BROWSER_TOOLS } from './registry'

type RegisterFn = (
  name: string,
  config: {
    description: string
    inputSchema?: ZodRawShape
    outputSchema?: ZodRawShape
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
  onToolExecuted?: (event: BrowserToolExecutionEvent) => void
  shouldLogToolRegistration?: () => boolean
  logger?: { info(message: string): void }
  source?: string
}

export interface BrowserToolExecutionEvent extends Record<string, unknown> {
  tool_name: string
  duration_ms: number
  success: boolean
  source: string
  error_message?: string
}

/** Registers the browser tool surface on an MCP server bound to one BrowserSession. */
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
        ...(tool.output && { outputSchema: tool.output.shape }),
        ...(tool.annotations && {
          annotations: tool.annotations as Record<string, unknown>,
        }),
      },
      async (args, extra) => {
        const startTime = performance.now()
        const duration = () => Math.round(performance.now() - startTime)
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
          options.onToolExecuted?.({
            tool_name: tool.name,
            duration_ms: duration(),
            success: !result.isError,
            source: options.source ?? 'mcp',
          })
          return {
            content: result.content,
            isError: result.isError,
            structuredContent: result.structuredContent,
          }
        } catch (error) {
          const errorText =
            error instanceof Error ? error.message : String(error)
          options.onToolExecuted?.({
            tool_name: tool.name,
            duration_ms: duration(),
            success: false,
            error_message: errorText,
            source: options.source ?? 'mcp',
          })
          return {
            content: [{ type: 'text' as const, text: errorText }],
            isError: true,
          }
        }
      },
    )
  }

  if (options.shouldLogToolRegistration?.()) {
    options.logger?.info(
      `Registered ${BROWSER_TOOLS.length} browser tools: ${BROWSER_TOOLS.map((t) => t.name).join(', ')}`,
    )
  }
}
