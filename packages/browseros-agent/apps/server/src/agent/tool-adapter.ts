import type { LanguageModelV2ToolResultOutput } from '@ai-sdk/provider'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import {
  type BrowserOutputFileAccess,
  withBrowserOutputFileAccess,
} from '@browseros/browser-mcp/output-file'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'
import {
  type ToolDefinition as BrowserToolDefinition,
  type ToolResult as BrowserToolResult,
  type ContentBlock,
  errorResult,
  executeTool as executeBrowserTool,
  throwIfAborted,
} from '@browseros/browser-mcp/tools/framework'
import { type ToolSet, tool } from 'ai'
import { logger } from '../lib/logger'
import { metrics } from '../lib/metrics'

export interface BrowserToolSetOptions {
  readOnly?: boolean
  outputFileAccess?: BrowserOutputFileAccess
}

interface ToolExecuteOptions {
  abortSignal?: AbortSignal
}

const BROWSER_TOOL_TIMEOUT_MS = 120_000

function summarizeBrowserToolParams(params: unknown): Record<string, unknown> {
  if (!params || typeof params !== 'object') {
    return { inputType: typeof params }
  }
  const input = params as Record<string, unknown>
  const summary: Record<string, unknown> = {
    argKeys: Object.keys(input).sort(),
  }
  if (typeof input.page === 'number') summary.page = input.page
  if (typeof input.action === 'string') summary.action = input.action
  if (typeof input.format === 'string') summary.format = input.format
  if (typeof input.timeoutMs === 'number') summary.timeoutMs = input.timeoutMs
  if (typeof input.selector === 'string') summary.selectorPresent = true
  if (typeof input.url === 'string') {
    try {
      summary.urlOrigin = new URL(input.url).origin
    } catch {
      summary.urlPresent = true
    }
  }
  return summary
}

function summarizeBrowserToolError(
  content: ContentBlock[],
): Record<string, unknown> {
  const textBlocks = content
    .filter(
      (item): item is ContentBlock & { type: 'text' } => item.type === 'text',
    )
    .map((item) => item.text)
  const text = textBlocks.join('\n')
  return {
    contentCount: content.length,
    textBlockCount: textBlocks.length,
    textLength: text.length,
    lineCount: text.length ? text.split('\n').length : 0,
  }
}

function withBrowserToolTimeout(signal?: AbortSignal): AbortSignal {
  const timeoutSignal = AbortSignal.timeout(BROWSER_TOOL_TIMEOUT_MS)
  if (!signal) return timeoutSignal

  const controller = new AbortController()
  const forwardAbort = (source: AbortSignal) => {
    if (source.aborted) {
      controller.abort(source.reason)
      return
    }
    source.addEventListener('abort', () => controller.abort(source.reason), {
      once: true,
    })
  }

  forwardAbort(signal)
  forwardAbort(timeoutSignal)
  return controller.signal
}

function contentToModelOutput(
  content: ContentBlock[],
): LanguageModelV2ToolResultOutput {
  const hasImages = content.some((c) => c.type === 'image')
  if (!hasImages) {
    const text = content
      .filter((c): c is ContentBlock & { type: 'text' } => c.type === 'text')
      .map((c) => c.text)
      .join('\n')
    return { type: 'text', value: text || 'Success' }
  }
  return {
    type: 'content',
    value: content.map((c) =>
      c.type === 'text'
        ? { type: 'text' as const, text: c.text }
        : { type: 'media' as const, data: c.data, mediaType: c.mimeType },
    ),
  }
}

/** Wraps the browser-core tool surface as AI SDK tools for the internal agent. */
export function buildBrowserToolSet(
  session: BrowserSession,
  options: BrowserToolSetOptions = {},
): ToolSet {
  const toolSet: ToolSet = {}

  for (const def of BROWSER_TOOLS) {
    toolSet[def.name] = tool({
      description: def.description,
      inputSchema: def.input,
      execute: async (params, executeOptions?: ToolExecuteOptions) => {
        const startTime = performance.now()
        const signal = withBrowserToolTimeout(executeOptions?.abortSignal)
        throwIfAborted(signal)
        const logBase = {
          toolName: def.name,
          source: 'chat',
        }
        logger.debug('Browser chat tool started', {
          ...logBase,
          args: summarizeBrowserToolParams(params),
          readOnly: Boolean(options.readOnly),
        })
        try {
          const result =
            readOnlyGuard(def, params, options) ??
            (await withBrowserOutputFileAccess(options.outputFileAccess, () =>
              executeBrowserTool(def, params as Record<string, unknown>, {
                session,
                signal,
              }),
            ))
          const durationMs = Math.round(performance.now() - startTime)
          metrics.log('tool_executed', {
            tool_name: def.name,
            duration_ms: durationMs,
            success: !result.isError,
            source: 'chat',
          })
          logger.debug('Browser chat tool completed', {
            ...logBase,
            durationMs,
            isError: Boolean(result.isError),
          })
          if (result.isError) {
            logger.info('Browser chat tool returned error', {
              ...logBase,
              durationMs,
              errorSummary: summarizeBrowserToolError(result.content),
            })
          }
          return { content: result.content, isError: result.isError ?? false }
        } catch (error) {
          logger.info('Browser chat tool threw', {
            ...logBase,
            durationMs: Math.round(performance.now() - startTime),
            error: error instanceof Error ? error.message : String(error),
          })
          throw error
        }
      },
      toModelOutput: ({ output }) => {
        const result = output as { content: ContentBlock[]; isError: boolean }
        if (result.isError) {
          const text = result.content
            .filter(
              (c): c is ContentBlock & { type: 'text' } => c.type === 'text',
            )
            .map((c) => c.text)
            .join('\n')
          return { type: 'error-text', value: text }
        }
        if (!result.content?.length) {
          return { type: 'text', value: 'Success' }
        }
        return contentToModelOutput(result.content)
      },
    })
  }

  return toolSet
}

function readOnlyGuard(
  def: BrowserToolDefinition,
  params: unknown,
  options: BrowserToolSetOptions,
): BrowserToolResult | null {
  if (!options.readOnly || def.name !== 'tabs') return null
  const action =
    params &&
    typeof params === 'object' &&
    'action' in params &&
    typeof params.action === 'string'
      ? params.action
      : 'list'
  if (action === 'list' || action === 'active') return null
  return errorResult('tabs: chat mode only supports action="list" or "active".')
}
