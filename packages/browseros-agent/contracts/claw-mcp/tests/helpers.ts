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

/**
 * The ref of the first element whose snapshot line matches, skipping lines that
 * match but carry no ref.
 *
 * A label can appear on a line of its own above the control it names, so the
 * first matching line is not always the actionable one. Taking only the first
 * match then yielded no ref and the case failed claiming the element was
 * missing while it was on the next line. A trailing space in the needle is
 * ignored, because whether an accessible name keeps one is the browser's
 * choice and has changed.
 */
export function refForLabel(snapshot: string, label: string): string {
  const wanted = label.replace(/\s+$/, '')
  for (const line of snapshot.split('\n')) {
    if (!line.includes(wanted)) continue
    const ref = line.match(/\[ref=(e\d+)\]/)?.[1]
    if (ref) return ref
  }
  throw new Error(`no ref for ${label} in:\n${snapshot.slice(0, 500)}`)
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
    // Two wordings on purpose: a link the server is reconnecting and a browser
    // that is not running need different advice, and both land in this class.
    'browser-down',
    /(browser session not connected.*start BrowserClaw|cdp not connected|not running or paired|browser link (lost .* |is down and ).*reconnecting|no link to the browser yet)/is,
  ],
  ['scheme-refused', /navigate refuses .* URLs; only http\(s\) is allowed/i],
]

export function errorClass(text: string): string {
  for (const [name, pattern] of ERROR_CLASSES) {
    if (pattern.test(text)) return name
  }
  return `other:${text.slice(0, 60).toLowerCase()}`
}
