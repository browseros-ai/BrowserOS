/**
 * @license AGPL-3.0-or-later
 * Copyright 2026 TRIOS
 *
 * TRIOS MCP Bridge - Tool Framework
 * Provides tool definition helpers for all bridge tools.
 */

import type { z } from 'zod'

export type ToolHandler<TInput, TOutput> = (
  args: TInput,
  ctx: any,
  response: ToolResponse<TOutput>,
) => Promise<void>

export interface ToolDefinition<TInput, TOutput> {
  name: string
  description: string
  approvalCategory?: string
  input: z.ZodType<TInput>
  output: z.ZodType<TOutput>
  handler: ToolHandler<TInput, TOutput>
}

export interface ToolResponse<T> {
  text: (message: string) => void
  data: (data: T) => void
  error: (message: string) => void
  image?: (data: string, mimeType?: string) => void
}

export function defineTool<TInput, TOutput>(
  def: ToolDefinition<TInput, TOutput>,
): ToolDefinition<TInput, TOutput> {
  return def
}
