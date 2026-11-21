/** Browser operations for a server that exposes the Neo SDK's Playwright run. */

import { expectOk, waitUntil } from '../helpers'
import { McpSession, textOf } from '../mcp-client'
import type { ContractServer } from '../rust-server'
import { type CaseContext, contractCases } from './cases'

async function connectAttached(
  server: ContractServer,
  clientName: string,
): Promise<McpSession> {
  const mcp = await McpSession.connect(server.baseUrl, clientName)
  // Health can precede the SDK's browser attachment. Read actual tabs through
  // run so an unavailable browser cannot look ready just because the realm runs.
  await waitUntil(
    async () => {
      const result = await mcp.callTool('run', {
        code: 'return (await context.pages()).map(p => p.tabId)',
      })
      return !result.isError && Array.isArray(result.structuredContent?.value)
    },
    'Rust server and SDK to attach to the browser',
    { timeoutMs: 30_000, intervalMs: 500 },
  )
  return mcp
}

async function openPage(
  run: Pick<CaseContext, 'mcp'>,
  url: string,
  session = run.mcp,
): Promise<number> {
  const result = await session.callTool('run', {
    code: `const p = await context.newPage(); await p.goto(${JSON.stringify(url)}); return p.tabId`,
  })
  expectOk(result, 'open fixture tab')
  const tabId = result.structuredContent?.value
  if (typeof tabId !== 'number') {
    throw new Error(`run did not return a tab id: ${textOf(result)}`)
  }
  return tabId
}

/**
 * This profile belongs entirely to the suite. Enumerating it also reclaims tabs
 * created before a script throws, which cannot be tracked from its return value.
 */
async function cleanupCase(
  run: Pick<CaseContext, 'browser' | 'mcp'>,
): Promise<void> {
  if (!(await run.browser.isRunning())) return
  expectOk(
    await run.mcp.callTool('run', {
      code: 'for (const p of await context.pages()) await p.close(); return null',
    }),
    'close case tabs',
  )
}

export const playwrightRunner = {
  name: 'playwright',
  cases: contractCases,
  connectAttached,
  openPage,
  cleanupCase,
  caseContext: (context: CaseContext) => context,
}
