/** Startup configuration, discovery and execution must agree on one run budget. */
import { expect } from 'bun:test'
import { writeFile } from 'node:fs/promises'
import { join } from 'node:path'
import type { ContractCase } from './cases'
import { expectError, expectOk, waitUntil } from './helpers'
import { McpSession, type McpToolInfo } from './mcp-client'
import { startRustServer } from './rust-server'

// Check the wire shape before inspecting untyped JSON schema fields.
function timeoutOf(tool: McpToolInfo | undefined) {
  const properties = tool?.inputSchema?.properties
  if (
    !properties ||
    typeof properties !== 'object' ||
    !('timeout' in properties)
  )
    throw new Error('run schema omitted timeout')
  const timeout = properties.timeout
  if (
    !timeout ||
    typeof timeout !== 'object' ||
    !('default' in timeout) ||
    !('maximum' in timeout)
  )
    throw new Error('run schema omitted its configured budget')
  return timeout
}

export const runBudgetCases: ContractCase[] = [
  {
    name: 'run budget: defaults to sixty seconds and exposes the limit',
    async run(ctx) {
      const run = (await ctx.mcp.listTools()).find(
        (tool) => tool.name === 'run',
      )
      expect(timeoutOf(run).default).toBe(60000)
      expect(timeoutOf(run).maximum).toBe(60000)
      // This real wait also crosses QuickJS's wall-clock interrupt, which virtual
      // time alone cannot exercise. The raw MCP client's 120s deadline leaves room.
      const completed = await ctx.mcp.callTool('run', {
        code: "await sleep(31000); return 'completed'",
      })
      expectOk(completed)
      expect(completed.structuredContent?.value).toBe('completed')
      expectError(
        await ctx.mcp.callTool('run', {
          code: "console.log('must not execute'); return 'must not execute'",
          timeout: 60001,
        }),
        'configured maximum of 60000ms',
      )
    },
  },
  {
    name: 'run budget: neo.toml controls hosted and desktop discovery and execution',
    async run(ctx) {
      for (const mode of ['0', '1']) {
        const config = join(ctx.scratchDir, `run-budget-${mode}.toml`)
        await writeFile(config, '[run]\nwall_timeout_seconds = 1\n')
        const server = await startRustServer(ctx.browser.cdpPort, {
          env: {
            BROWSERCLAW_CONFIG_FILE: config,
            BROWSERCLAW_HOSTED_MODE: mode,
          },
        })
        let mcp: McpSession | undefined
        try {
          mcp = await McpSession.connect(server.baseUrl, 'run-budget')
          const attached = mcp
          await waitUntil(
            async () =>
              !(await attached.callTool('tabs', { action: 'list' })).isError,
            'configured server to attach',
          )
          const run = (await mcp.listTools()).find(
            (tool) => tool.name === 'run',
          )
          expect(timeoutOf(run).default).toBe(1000)
          expect(timeoutOf(run).maximum).toBe(1000)
          const malformed = await mcp.callTool('run', {
            code: "console.log('must not execute'); return 'must not execute'",
            timeout: null,
          })
          expect(malformed.isError).toBe(true)
          expect(JSON.stringify(malformed)).not.toContain('must not execute')
          expectOk(await mcp.callTool('run', { code: 'return 42' }))
          const expired = await mcp.callTool('run', {
            code: "console.log('started'); await sleep(1500); return 'late'",
          })
          expectError(expired, 'run exceeded 1000ms')
          expect(expired.structuredContent?.logs).toEqual(['started'])
          expectError(
            await mcp.callTool('run', {
              code: 'await sleep(500)',
              timeout: 50,
            }),
            'run exceeded 50ms',
          )
          const rejected = await mcp.callTool('run', {
            code: "console.log('must not execute'); return 'must not execute'",
            timeout: 1001,
          })
          expectError(rejected, 'configured maximum of 1000ms')
          expect(JSON.stringify(rejected)).not.toContain('must not execute')
        } finally {
          await mcp?.close().catch(() => {})
          await server.stop()
        }
      }
    },
  },
]
