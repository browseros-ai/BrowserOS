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

interface BrowserToolLogger {
  debug?(message: string, meta?: Record<string, unknown>): void
  info?(message: string, meta?: Record<string, unknown>): void
}

export interface BrowserToolRegistrationOptions {
  outputFileAccess?: BrowserOutputFileAccess
  onToolExecutionStart?: (event: BrowserToolLifecycleEvent) => void
  onToolExecutionEnd?: (event: BrowserToolLifecycleEvent) => void
  onToolExecuted?: (event: BrowserToolExecutionEvent) => void
  shouldLogToolRegistration?: () => boolean
  logger?: BrowserToolLogger
  source?: string
}

export interface BrowserToolLifecycleEvent extends Record<string, unknown> {
  tool_name: string
  source: string
}

export interface BrowserToolExecutionEvent extends Record<string, unknown> {
  tool_name: string
  duration_ms: number
  success: boolean
  source: string
  error_message?: string
}

function summarizeBrowserToolArgs(
  args: Record<string, unknown>,
): Record<string, unknown> {
  const summary: Record<string, unknown> = {
    argKeys: Object.keys(args).sort(),
  }
  if (typeof args.page === 'number') summary.page = args.page
  if (typeof args.action === 'string') summary.action = args.action
  if (typeof args.format === 'string') summary.format = args.format
  if (typeof args.timeoutMs === 'number') summary.timeoutMs = args.timeoutMs
  if (typeof args.timeout === 'number') summary.timeout = args.timeout
  if (typeof args.selector === 'string') summary.selectorPresent = true
  if (typeof args.url === 'string') {
    try {
      summary.urlOrigin = new URL(args.url).origin
    } catch {
      summary.urlPresent = true
    }
  }
  return summary
}

function summarizeText(text: string): Record<string, unknown> {
  return {
    textLength: text.length,
    lineCount: text.length ? text.split('\n').length : 0,
  }
}

function resultTextSummary(
  content: unknown,
): Record<string, unknown> | undefined {
  if (!Array.isArray(content)) return undefined
  const textBlocks = content
    .filter(
      (item): item is { type: 'text'; text: string } =>
        typeof item === 'object' &&
        item !== null &&
        'type' in item &&
        item.type === 'text' &&
        'text' in item &&
        typeof item.text === 'string',
    )
    .map((item) => item.text)
  if (textBlocks.length === 0) {
    return {
      contentCount: content.length,
      textBlockCount: 0,
      textLength: 0,
      lineCount: 0,
    }
  }
  return {
    contentCount: content.length,
    textBlockCount: textBlocks.length,
    ...summarizeText(textBlocks.join('\n')),
  }
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
        const source = options.source ?? 'mcp'
        const startTime = performance.now()
        const duration = () => Math.round(performance.now() - startTime)
        const logBase = {
          toolName: tool.name,
          source,
        }
        const lifecycleEvent = {
          tool_name: tool.name,
          source,
        }
        options.logger?.debug?.('MCP browser tool started', {
          ...logBase,
          args: summarizeBrowserToolArgs(args),
          defaultWindowId: defaults.defaultWindowId,
          defaultTabGroupId: defaults.defaultTabGroupId,
        })
        options.onToolExecutionStart?.(lifecycleEvent)
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
            source,
          })
          const durationMs = duration()
          const errorSummary = result.isError
            ? resultTextSummary(result.content)
            : undefined
          options.logger?.debug?.('MCP browser tool completed', {
            ...logBase,
            durationMs,
            isError: Boolean(result.isError),
            hasStructuredContent: result.structuredContent !== undefined,
          })
          if (result.isError) {
            options.logger?.info?.('MCP browser tool returned error', {
              ...logBase,
              durationMs,
              errorSummary,
            })
          }
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
            source,
          })
          options.logger?.info?.('MCP browser tool threw', {
            ...logBase,
            durationMs: duration(),
            error: errorText,
          })
          return {
            content: [{ type: 'text' as const, text: errorText }],
            isError: true,
          }
        } finally {
          options.onToolExecutionEnd?.(lifecycleEvent)
        }
      },
    )
  }

  if (options.shouldLogToolRegistration?.()) {
    options.logger?.info?.('Registered browser MCP tools', {
      count: BROWSER_TOOLS.length,
      toolNames: BROWSER_TOOLS.map((t) => t.name),
      source: options.source ?? 'mcp',
    })
  }
}
