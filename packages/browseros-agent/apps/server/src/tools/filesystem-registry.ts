/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Filesystem tools registered for MCP exposure.
 *
 * Wraps the ai-SDK tool factories from ./filesystem/ into defineTool
 * so they participate in the unified ToolRegistry / MCP server.
 */

import { z } from 'zod'
import { createBashTool } from './filesystem/bash'
import { createEditTool } from './filesystem/edit'
import { createFindTool } from './filesystem/find'
import { createGrepTool } from './filesystem/grep'
import { createLsTool } from './filesystem/ls'
import { createReadTool } from './filesystem/read'
import type { FilesystemToolResult } from './filesystem/utils'
import { createWriteTool } from './filesystem/write'
import { defineToolWithCategory } from './framework'

const defineFilesystemTool = defineToolWithCategory('data-modification')

function getCwd(ctx: { directories: { workingDir?: string } }): string {
  return ctx.directories.workingDir ?? process.cwd()
}

// biome-ignore lint/suspicious/noExplicitAny: aligning with ai-sdk ToolExecutionOptions
interface ToolExecutionOptions {
  toolCallId: string
  messages: any[]
  abortSignal?: AbortSignal
}

async function runTool<TArgs extends Record<string, unknown>>(
  tool: { execute?: (args: TArgs, options: ToolExecutionOptions) => unknown },
  args: TArgs,
): Promise<FilesystemToolResult> {
  const result = await tool.execute!(args, {
    toolCallId: crypto.randomUUID(),
    messages: [],
    abortSignal: undefined,
  })
  return result as FilesystemToolResult
}

export const filesystem_bash = defineFilesystemTool({
  name: 'filesystem_bash',
  description:
    'Execute a shell command and return its output. Commands run in a shell (sh/bash on Unix, cmd on Windows). Output is truncated to the last 2000 lines if too large.',
  input: z.object({
    command: z.string().describe('Shell command to execute'),
    timeout: z
      .number()
      .optional()
      .describe('Timeout in seconds (default: 120)'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createBashTool(getCwd(ctx))
    const result = await runTool(tool, args)
    if (result.isError) response.error(result.text)
    else response.text(result.text)
  },
})

export const filesystem_ls = defineFilesystemTool({
  name: 'filesystem_ls',
  description:
    'List directory contents. Shows directories (with trailing /) first, then files with sizes. Entries are sorted alphabetically.',
  input: z.object({
    path: z
      .string()
      .optional()
      .describe('Directory path (default: working directory)'),
    limit: z
      .number()
      .optional()
      .describe('Maximum entries to return (default: 500)'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createLsTool(getCwd(ctx))
    const result = await runTool(tool, args)
    response.text(result.text)
  },
})

export const filesystem_read = defineFilesystemTool({
  name: 'filesystem_read',
  description:
    'Read a file from the filesystem. Returns text content with line numbers, or image data for image files. Text reads are limited to 100 lines and 10 000 characters per call. Use offset and limit to paginate through large files.',
  input: z.object({
    path: z
      .string()
      .describe('File path (relative to working directory or absolute)'),
    offset: z.number().optional().describe('Starting line number (1-indexed)'),
    limit: z
      .number()
      .int()
      .positive()
      .optional()
      .describe('Maximum number of lines to read'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createReadTool(getCwd(ctx))
    const result = await runTool(tool, args)
    if (result.images?.length) {
      response.text(result.text ?? '')
      for (const img of result.images) {
        response.image(img.data, img.mimeType)
      }
    } else {
      response.text(result.text)
    }
  },
})

export const filesystem_write = defineFilesystemTool({
  name: 'filesystem_write',
  description:
    "Create or overwrite a file. Automatically creates parent directories if they don't exist. Use this to create new files or completely replace file contents.",
  input: z.object({
    path: z
      .string()
      .describe('File path (relative to working directory or absolute)'),
    content: z.string().describe('Complete file content to write'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createWriteTool(getCwd(ctx))
    const result = await runTool(tool, args)
    response.text(result.text)
  },
})

export const filesystem_edit = defineFilesystemTool({
  name: 'filesystem_edit',
  description:
    'Make a targeted edit to a file by replacing an exact string match. The old_string must match exactly one location in the file. If exact match fails, a whitespace-tolerant match is attempted.',
  input: z.object({
    path: z
      .string()
      .describe('File path (relative to working directory or absolute)'),
    old_string: z.string().describe('Exact text to find in the file'),
    new_string: z.string().describe('Replacement text'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createEditTool(getCwd(ctx))
    const result = await runTool(tool, args)
    if (result.isError) response.error(result.text)
    else response.text(result.text)
  },
})

export const filesystem_find = defineFilesystemTool({
  name: 'filesystem_find',
  description:
    'Find files matching a glob pattern. Searches recursively, skipping common build directories (node_modules, .git, dist, etc.). Returns relative file paths.',
  input: z.object({
    pattern: z
      .string()
      .describe('Glob pattern (e.g., "*.ts", "**/*.json", "src/**/*.test.ts")'),
    path: z
      .string()
      .optional()
      .describe('Directory to search (default: working directory)'),
    limit: z.number().optional().describe('Maximum results (default: 1000)'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createFindTool(getCwd(ctx))
    const result = await runTool(tool, args)
    response.text(result.text)
  },
})

export const filesystem_grep = defineFilesystemTool({
  name: 'filesystem_grep',
  description:
    'Search file contents using a regular expression. Returns matching lines with file paths and line numbers. Searches recursively, skipping binary files and common build directories (node_modules, .git, dist, etc.).',
  input: z.object({
    pattern: z
      .string()
      .describe(
        'Search pattern (regex by default, or literal string if literal=true)',
      ),
    path: z
      .string()
      .optional()
      .describe('Directory or file to search (default: working directory)'),
    glob: z
      .string()
      .optional()
      .describe('Filter files by glob pattern (e.g., "*.ts", "*.{js,jsx}")'),
    ignore_case: z.boolean().optional().describe('Case-insensitive search'),
    literal: z
      .boolean()
      .optional()
      .describe('Treat pattern as a literal string, not regex'),
    context: z
      .number()
      .optional()
      .describe('Lines of context before and after each match'),
    limit: z
      .number()
      .optional()
      .describe('Maximum matches to return (default: 100)'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createGrepTool(getCwd(ctx))
    const result = await runTool(tool, args)
    response.text(result.text)
  },
})

export const fs_read = defineFilesystemTool({
  name: 'fs_read',
  description:
    'Read a file from the filesystem. Returns text content with line numbers, or image data for image files. Text reads are limited to 100 lines and 10 000 characters per call. Use offset and limit to paginate through large files.',
  input: z.object({
    path: z
      .string()
      .describe('File path (relative to working directory or absolute)'),
    offset: z.number().optional().describe('Starting line number (1-indexed)'),
    limit: z
      .number()
      .int()
      .positive()
      .optional()
      .describe('Maximum number of lines to read'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createReadTool(getCwd(ctx))
    const result = await runTool(tool, args)
    if (result.images?.length) {
      response.text(result.text ?? '')
      for (const img of result.images) {
        response.image(img.data, img.mimeType)
      }
    } else {
      response.text(result.text)
    }
  },
})

export const fs_write = defineFilesystemTool({
  name: 'fs_write',
  description:
    "Create or overwrite a file. Automatically creates parent directories if they don't exist. Use this to create new files or completely replace file contents.",
  input: z.object({
    path: z
      .string()
      .describe('File path (relative to working directory or absolute)'),
    content: z.string().describe('Complete file content to write'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createWriteTool(getCwd(ctx))
    const result = await runTool(tool, args)
    response.text(result.text)
  },
})

export const fs_list = defineFilesystemTool({
  name: 'fs_list',
  description:
    'List directory contents. Shows directories (with trailing /) first, then files with sizes. Entries are sorted alphabetically.',
  input: z.object({
    path: z
      .string()
      .optional()
      .describe('Directory path (default: working directory)'),
    limit: z
      .number()
      .optional()
      .describe('Maximum entries to return (default: 500)'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createLsTool(getCwd(ctx))
    const result = await runTool(tool, args)
    response.text(result.text)
  },
})

export const shell_execute = defineFilesystemTool({
  name: 'shell_execute',
  description:
    'Execute a shell command and return its output. Commands run in a shell (sh/bash on Unix, cmd on Windows). Output is truncated to the last 2000 lines if too large.',
  input: z.object({
    command: z.string().describe('Shell command to execute'),
    timeout: z
      .number()
      .optional()
      .describe('Timeout in seconds (default: 120)'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createBashTool(getCwd(ctx))
    const result = await runTool(tool, args)
    if (result.isError) response.error(result.text)
    else response.text(result.text)
  },
})

export const fs_edit = defineFilesystemTool({
  name: 'fs_edit',
  description:
    'Make a targeted edit to a file by replacing an exact string match. The old_string must match exactly one location in the file. If exact match fails, a whitespace-tolerant match is attempted.',
  input: z.object({
    path: z
      .string()
      .describe('File path (relative to working directory or absolute)'),
    old_string: z.string().describe('Exact text to find in the file'),
    new_string: z.string().describe('Replacement text'),
  }),
  handler: async (args, ctx, response) => {
    const tool = createEditTool(getCwd(ctx))
    const result = await runTool(tool, args)
    if (result.isError) response.error(result.text)
    else response.text(result.text)
  },
})
