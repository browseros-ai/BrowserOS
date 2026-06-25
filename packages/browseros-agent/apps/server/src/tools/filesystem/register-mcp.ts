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
        const result = await tool.execute(args, { signal: extra?.signal })
        if (result.isError) {
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
