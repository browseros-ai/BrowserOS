import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import { logger } from '../../../lib/logger'
import { metrics } from '../../../lib/metrics'
import type { ToolExecutionObserver } from '../../../monitoring/observer'
import { executeTool, type ToolContext } from '../../../tools/framework'
import type { ToolRegistry } from '../../../tools/tool-registry'

export function registerTools(
  mcpServer: McpServer,
  registry: ToolRegistry,
  ctx: ToolContext & { observer?: ToolExecutionObserver },
): void {
  for (const tool of registry.all()) {
    const handler = async (
      args: Record<string, unknown>,
      extra: { signal: AbortSignal },
    ) => {
      const startTime = performance.now()
      const toolCallId = crypto.randomUUID()

      try {
        logger.info(`${tool.name} request: ${JSON.stringify(args, null, '  ')}`)
        await ctx.observer?.onToolStart({
          toolCallId,
          toolName: tool.name,
          source: 'browser-tool',
          args,
        })

        const result = await executeTool(tool, args, ctx, extra.signal)

        metrics.log('tool_executed', {
          tool_name: tool.name,
          duration_ms: Math.round(performance.now() - startTime),
          success: !result.isError,
          source: 'mcp',
        })

        await ctx.observer?.onToolEnd({
          toolCallId,
          output: result.structuredContent ?? result.content,
          error: result.isError ? 'Tool returned isError=true' : undefined,
        })

        return {
          content: result.content,
          isError: result.isError,
          structuredContent: result.structuredContent,
        }
      } catch (error) {
        const errorText = error instanceof Error ? error.message : String(error)

        metrics.log('tool_executed', {
          tool_name: tool.name,
          duration_ms: Math.round(performance.now() - startTime),
          success: false,
          error_message: errorText,
          source: 'mcp',
        })

        await ctx.observer?.onToolEnd({
          toolCallId,
          error: errorText,
        })

        return {
          content: [{ type: 'text' as const, text: errorText }],
          isError: true,
        }
      }
    }

    mcpServer.registerTool(
      tool.name,
      {
        description: tool.description,
        inputSchema: tool.input as unknown as Record<string, never>,
        outputSchema: tool.output as unknown as Record<string, never>,
      },
      handler,
    )
  }

  logger.info(
    `Registered ${registry.names().length} tools: ${registry.names().join(', ')}`,
  )
}
