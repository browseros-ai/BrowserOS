import type { BrowserSession } from '@browseros/browser-core/core/session'
import type { McpServer } from '@modelcontextprotocol/server'
import { z } from 'zod/v4'
import {
  executeTool,
  type ToolContext,
  type ToolDefinition,
  type ToolResult,
} from './framework'
import {
  type BrowserOutputFileAccess,
  withBrowserOutputFileAccess,
} from './output-file'
import { BROWSER_TOOLS } from './registry'

// The `_meta` key the server-minted session handle is returned under, per the
// MCP `_meta` convention. The handle deliberately never rides in
// `structuredContent`: a client that prefers structuredContent (e.g. Claude
// Code) would otherwise receive only the handle for the schemaless tools that
// emit no structured payload, and it would collide with `run`'s declared output
// schema. See issue #2651 (and #2513, the same fix on the neo server).
const SESSION_META_KEY = 'com.browseros/session'

const SESSION_ARG_DESCRIPTION =
  'Opaque session handle for this browser session. The server returns it in every tool result under `_meta` at the key `com.browseros/session`; read it from there and pass it back as this `session` argument on every later call to keep the same browser session and its tab ownership. Omit it only on the first call to start a new session.'

function resolveSessionHandle(
  args: Record<string, unknown>,
  enabled: boolean | undefined,
): { sessionHandle?: string; toolArgs: Record<string, unknown> } {
  if (!enabled) return { toolArgs: args }
  const provided = typeof args.session === 'string' ? args.session.trim() : ''
  const sessionHandle = provided.length > 0 ? provided : crypto.randomUUID()
  const toolArgs = { ...args }
  delete toolArgs.session
  return { sessionHandle, toolArgs }
}

/**
 * Assembles the MCP tool result. The session handle rides in `_meta`, never in
 * `structuredContent`, so it never replaces a schemaless tool's result nor
 * collides with a declared output schema (issue #2651). Structured content is
 * emitted only when the caller opted in or the tool declares an output schema.
 */
function buildToolResult(
  result: ToolResult,
  includeStructured: boolean,
  hasOutputSchema: boolean,
  sessionHandle: string | undefined,
): {
  content: unknown
  isError?: boolean
  structuredContent?: unknown
  _meta?: Record<string, unknown>
} {
  const structuredContent =
    includeStructured || hasOutputSchema ? result.structuredContent : undefined
  return {
    content: result.content,
    isError: result.isError,
    ...(structuredContent !== undefined && { structuredContent }),
    ...(sessionHandle !== undefined && {
      _meta: { [SESSION_META_KEY]: sessionHandle },
    }),
  }
}

type RegisterFn = (
  name: string,
  config: {
    description: string
    inputSchema?: unknown
    outputSchema?: unknown
    annotations?: Record<string, unknown>
  },
  handler: (
    args: Record<string, unknown>,
    extra?: { mcpReq?: { signal?: AbortSignal } },
  ) => Promise<{
    content: unknown
    isError?: boolean
    structuredContent?: unknown
    _meta?: Record<string, unknown>
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
  /** Tool catalog exposed by this server. Defaults to the full browser surface. */
  tools?: readonly ToolDefinition[]
  /**
   * Optional policy-aware executor. The server runtime uses this seam to keep
   * guards, output grants, and post-execution effects on the authoritative side
   * of the HTTP boundary.
   */
  executor?: BrowserToolExecutor
  includeStructuredContent?: boolean
  outputFileAccess?: BrowserOutputFileAccess
  onToolExecutionStart?: (event: BrowserToolLifecycleEvent) => void
  onToolExecutionEnd?: (event: BrowserToolLifecycleEvent) => void
  onToolExecuted?: (event: BrowserToolExecutionEvent) => void
  shouldLogToolRegistration?: () => boolean
  logger?: BrowserToolLogger
  source?: string
  /** When set, expose an optional server-minted session handle argument for caller session identity (2026-07-28). */
  sessionIdentity?: boolean
}

export type BrowserToolExecutor = (
  tool: ToolDefinition,
  args: Record<string, unknown>,
  context: ToolContext,
) => Promise<ToolResult>

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
  const tools = options.tools ?? BROWSER_TOOLS

  for (const tool of tools) {
    const inputSchema = options.sessionIdentity
      ? tool.input.extend({
          session: z.string().optional().describe(SESSION_ARG_DESCRIPTION),
        })
      : tool.input
    register(
      tool.name,
      {
        description: tool.description,
        inputSchema,
        ...(tool.output && { outputSchema: tool.output }),
        ...(tool.annotations && {
          annotations: tool.annotations as Record<string, unknown>,
        }),
      },
      async (args, extra) => {
        const source = options.source ?? 'mcp'
        const startTime = performance.now()
        const duration = () => Math.round(performance.now() - startTime)
        const { sessionHandle, toolArgs } = resolveSessionHandle(
          args,
          options.sessionIdentity,
        )
        const sessionField =
          sessionHandle !== undefined ? { session: sessionHandle } : undefined
        const logBase = {
          toolName: tool.name,
          source,
          ...sessionField,
        }
        const lifecycleEvent = {
          tool_name: tool.name,
          source,
        }
        options.logger?.debug?.('MCP browser tool started', {
          ...logBase,
          args: summarizeBrowserToolArgs(toolArgs),
          defaultWindowId: defaults.defaultWindowId,
          defaultTabGroupId: defaults.defaultTabGroupId,
        })
        options.onToolExecutionStart?.(lifecycleEvent)
        try {
          const context = {
            session,
            ...defaults,
            signal: extra?.mcpReq?.signal,
          }
          const result = options.executor
            ? await options.executor(tool, toolArgs, context)
            : await withBrowserOutputFileAccess(options.outputFileAccess, () =>
                executeTool(tool, toolArgs, context),
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
          const toolResult = buildToolResult(
            result,
            options.includeStructuredContent ?? true,
            tool.output !== undefined,
            sessionHandle,
          )
          options.logger?.debug?.('MCP browser tool completed', {
            ...logBase,
            durationMs,
            isError: Boolean(result.isError),
            hasStructuredContent: toolResult.structuredContent !== undefined,
          })
          if (result.isError) {
            options.logger?.info?.('MCP browser tool returned error', {
              ...logBase,
              durationMs,
              errorSummary,
            })
          }
          return toolResult
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
          return buildToolResult(
            { content: [{ type: 'text', text: errorText }], isError: true },
            options.includeStructuredContent ?? true,
            tool.output !== undefined,
            sessionHandle,
          )
        } finally {
          options.onToolExecutionEnd?.(lifecycleEvent)
        }
      },
    )
  }

  if (options.shouldLogToolRegistration?.()) {
    options.logger?.info?.('Registered browser MCP tools', {
      count: tools.length,
      toolNames: tools.map((t) => t.name),
      source: options.source ?? 'mcp',
    })
  }
}
