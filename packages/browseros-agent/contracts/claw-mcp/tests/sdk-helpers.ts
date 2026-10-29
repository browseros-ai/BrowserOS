/** Shared wire helpers for SDK parity cases. Postconditions are read through
 * the granular evaluate tool so a successful SDK call alone cannot prove a fix. */
import { expect } from 'bun:test'
import type { CaseContext } from './cases'
import { expectError, expectOk } from './helpers'
import type { McpToolResult } from './mcp-client'

export type Surface = 'act' | 'run'
type WireValue =
  | string
  | number
  | boolean
  | null
  | WireValue[]
  | { [key: string]: WireValue }

export function evalValue(
  ctx: CaseContext,
  page: number,
  code: string,
): Promise<WireValue>
export function evalValue<T>(
  ctx: CaseContext,
  page: number,
  code: string,
): Promise<T>
export async function evalValue(
  ctx: CaseContext,
  page: number,
  code: string,
): Promise<unknown> {
  const result = await ctx.mcp.callTool('evaluate_in_page', {
    page,
    // The wire intentionally strips evaluate's structured metadata. Wrapping
    // avoids confusing a raw string such as "75" with the JSON number 75.
    code: `const fixtureValue = await (async () => { ${code}\n })(); return { fixtureValue }`,
  })
  const text = expectOk(result, 'read fixture state')
  const payload = text.match(
    /\[UNTRUSTED_PAGE_CONTENT nonce=([a-f0-9]{16})[^\n]*\n([\s\S]*?)\n\[END_UNTRUSTED_PAGE_CONTENT nonce=\1\]/,
  )?.[2]
  if (!payload) throw new Error(`evaluate omitted its fenced value: ${text}`)
  return JSON.parse(payload).fixtureValue
}

export function runValue(
  ctx: CaseContext,
  page: number,
  code: string,
): Promise<WireValue>
export function runValue<T>(
  ctx: CaseContext,
  page: number,
  code: string,
): Promise<T>
export async function runValue(
  ctx: CaseContext,
  page: number,
  code: string,
): Promise<unknown> {
  const result = await ctx.mcp.callTool('run', {
    code: `const p = browser.page(${page}); ${code}`,
  })
  expectOk(result, 'run SDK')
  expect(result.structuredContent?.ok).toBe(true)
  return result.structuredContent?.value
}

export function errorIncludes(result: McpToolResult, message: string): void {
  expect(expectError(result)).toContain(message)
}

export async function inputCall(
  ctx: CaseContext,
  page: number,
  surface: Surface,
  method: string,
  ref: string,
  value?: string,
): Promise<McpToolResult> {
  if (surface === 'act') {
    return ctx.mcp.callTool('act', { page, kind: method, ref, value })
  }
  return ctx.mcp.callTool('run', {
    code: `await browser.page(${page}).${method}(${JSON.stringify(ref)}${value === undefined ? '' : `, ${JSON.stringify(value)}`})`,
  })
}
