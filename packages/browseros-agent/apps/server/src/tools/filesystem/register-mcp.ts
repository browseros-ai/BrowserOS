/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Bridges the AI-SDK filesystem toolset onto the laptop's MCP server.
 * The AI-SDK and MCP tool registries are independent (the local agent
 * loop never dials /mcp for its own tools), so registering here only
 * affects external remote-harness MCP callers.
 */

import type { BrowserOutputFileAccess } from '@browseros/browser-mcp/output-file'
import type { McpServer } from '@modelcontextprotocol/sdk/server/mcp.js'
import type { z } from 'zod'
import { logger } from '../../lib/logger'
import { shouldLogToolRegistration } from '../registration-log-sampling'
import { buildFilesystemToolSet } from './build-toolset'
import type { FilesystemToolResult } from './utils'

// Shape we depend on from the AI-SDK `tool({...})` return value at
// runtime. Asserted via a single cast so the rest of the file is typed.
interface AiSdkToolLike {
  description?: string
  inputSchema: z.ZodObject<z.ZodRawShape>
  execute: (
    args: Record<string, unknown>,
    options: { signal?: AbortSignal },
  ) => Promise<FilesystemToolResult>
}

type McpRegisterFn = (
  name: string,
  config: { description: string; inputSchema: z.ZodRawShape },
  handler: (
    args: Record<string, unknown>,
    extra?: { signal?: AbortSignal },
  ) => Promise<{
    content: Array<
      | { type: 'text'; text: string }
      | { type: 'image'; data: string; mimeType: string }
    >
    isError?: boolean
  }>,
) => void

export interface RegisterFilesystemMcpToolsOptions {
  outputFileAccess?: BrowserOutputFileAccess
}

function summarizeFilesystemArgs(
  args: Record<string, unknown>,
): Record<string, unknown> {
  const summary: Record<string, unknown> = {
    argKeys: Object.keys(args).sort(),
  }
  if (typeof args.path === 'string') summary.path = args.path
  if (typeof args.pattern === 'string')
    summary.patternLength = args.pattern.length
  if (typeof args.glob === 'string') summary.glob = args.glob
  if (typeof args.limit === 'number') summary.limit = args.limit
  if (typeof args.command === 'string') {
    summary.commandLength = args.command.length
  }
  return summary
}

function summarizeFilesystemErrorText(
  toolName: string,
  text: string,
): Record<string, unknown> {
  const summary: Record<string, unknown> = {
    textLength: text.length,
    lineCount: text.length ? text.split('\n').length : 0,
  }
  if (toolName !== 'filesystem_bash') return summary

  const exitCodeMatch = text.match(/\n\n\[Exit code: (-?\d+)\]\s*$/)
  if (exitCodeMatch) {
    summary.exitCode = Number(exitCodeMatch[1])
  }
  const timeoutMatch = text.match(/^Command timed out after ([0-9.]+)s\b/)
  if (timeoutMatch) {
    summary.timedOut = true
    summary.timeoutSeconds = Number(timeoutMatch[1])
  }
  return summary
}

export function registerFilesystemMcpTools(
  server: McpServer,
  cwd: string,
  options: RegisterFilesystemMcpToolsOptions = {},
): void {
  const register = server.registerTool.bind(server) as unknown as McpRegisterFn
  const tools = buildFilesystemToolSet(cwd, {
    read: {
      allowedOutputPaths: options.outputFileAccess?.paths,
      requireAllowedOutputPath: Boolean(options.outputFileAccess),
    },
  }) as unknown as Record<string, AiSdkToolLike>

  for (const [name, tool] of Object.entries(tools)) {
    register(
      name,
      {
        description: tool.description ?? '',
        inputSchema: tool.inputSchema.shape,
      },
      async (args, extra) => {
        const startTime = performance.now()
        const duration = () => Math.round(performance.now() - startTime)
        const logBase = {
          toolName: name,
          source: 'mcp',
          cwd,
        }
        logger.debug('MCP filesystem tool started', {
          ...logBase,
          args: summarizeFilesystemArgs(args),
        })
        let result: FilesystemToolResult
        try {
          result = await tool.execute(args, { signal: extra?.signal })
        } catch (error) {
          const errorText =
            error instanceof Error ? error.message : String(error)
          logger.info('MCP filesystem tool threw', {
            ...logBase,
            durationMs: duration(),
            error: errorText,
          })
          throw error
        }
        logger.debug('MCP filesystem tool completed', {
          ...logBase,
          durationMs: duration(),
          isError: Boolean(result.isError),
          imageCount: result.images?.length ?? 0,
        })
        if (result.isError) {
          logger.info('MCP filesystem tool returned error', {
            ...logBase,
            durationMs: duration(),
            errorSummary: summarizeFilesystemErrorText(name, result.text),
          })
          return {
            content: [{ type: 'text', text: result.text }],
            isError: true,
          }
        }
        const content: Array<
          | { type: 'text'; text: string }
          | { type: 'image'; data: string; mimeType: string }
        > = [{ type: 'text', text: result.text || 'Success' }]
        if (result.images?.length) {
          for (const img of result.images) {
            content.push({
              type: 'image',
              data: img.data,
              mimeType: img.mimeType,
            })
          }
        }
        return { content }
      },
    )
  }

  if (shouldLogToolRegistration()) {
    logger.info(
      `Registered ${Object.keys(tools).length} filesystem MCP tools scoped to ${cwd}`,
    )
  }
}
