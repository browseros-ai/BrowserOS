/** Shared assertions and polling utilities for contract cases. */

import { type McpToolResult, textOf } from './mcp-client'
import type { ContractServer } from './rust-server'

/** Returns the result text, failing loudly when the tool errored. */
export function expectOk(result: McpToolResult, context = 'tool call'): string {
  const text = textOf(result)
  if (result.isError) {
    throw new Error(`${context} unexpectedly failed: ${text}`)
  }
  return text
}

/** Returns the result text, failing loudly when the tool succeeded. */
export function expectError(
  result: McpToolResult,
  context = 'tool call',
): string {
  if (!result.isError) {
    throw new Error(
      `${context} unexpectedly succeeded: ${textOf(result).slice(0, 300)}`,
    )
  }
  return textOf(result)
}

/** Condition-based waiting — the suite never sleeps blind. */
export async function waitUntil(
  condition: () => Promise<boolean> | boolean,
  label: string,
  { timeoutMs = 15_000, intervalMs = 200 } = {},
): Promise<void> {
  const deadline = Date.now() + timeoutMs
  while (Date.now() < deadline) {
    if (await condition()) return
    await Bun.sleep(intervalMs)
  }
  throw new Error(`timed out after ${timeoutMs}ms waiting for ${label}`)
}

export async function apiGet(
  server: ContractServer,
  path: string,
): Promise<Response> {
  return await fetch(`${server.baseUrl}${path}`, {
    signal: AbortSignal.timeout(10_000),
  })
}
