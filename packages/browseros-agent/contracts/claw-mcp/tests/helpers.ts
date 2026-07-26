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

export function parsePageId(result: McpToolResult): number {
  const structured = result.structuredContent as { page?: number } | undefined
  if (typeof structured?.page === 'number') return structured.page
  const match = textOf(result).match(/opened page (\d+)/)
  if (!match) {
    throw new Error(
      `could not find a page id in: ${textOf(result).slice(0, 200)}`,
    )
  }
  return Number(match[1])
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

const ERROR_CLASSES: Array<[string, RegExp]> = [
  ['stale-ref', /stale ref .*take a new snapshot/i],
  ['unknown-ref', /unknown ref .*take a new snapshot/i],
  ['gone-element', /not found in dom.*take a new snapshot/i],
  ['not-owned', /is (not )?owned by .*tabs new/i],
  [
    'browser-down',
    /(browser session not connected.*start BrowserClaw|cdp not connected|not running or paired)/is,
  ],
  ['scheme-refused', /navigate refuses .* URLs; only http\(s\) is allowed/i],
]

export function errorClass(text: string): string {
  for (const [name, pattern] of ERROR_CLASSES) {
    if (pattern.test(text)) return name
  }
  return `other:${text.slice(0, 60).toLowerCase()}`
}
