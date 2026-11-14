/** Wire assertions for the SDK's Playwright `run`. The SDK returns the script's
 * JSON value directly, without the retired browser SDK's `ok` envelope. */
import type { CaseContext } from './cases'
import { expectOk } from './helpers'

export async function runValue(
  ctx: Pick<CaseContext, 'mcp'>,
  code: string,
): Promise<unknown> {
  const result = await ctx.mcp.callTool('run', { code })
  expectOk(result, 'Playwright run')
  if (!result.structuredContent || !('value' in result.structuredContent)) {
    throw new Error('Playwright run omitted structuredContent.value')
  }
  return result.structuredContent.value
}
