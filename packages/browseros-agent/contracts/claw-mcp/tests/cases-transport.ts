/**
 * Transport & session-plumbing cases (cross-cutting invariant [7]):
 * the `/mcp` endpoint itself — catalog exposure, browser-request
 * hygiene, session-id handling, DELETE teardown + audit tie-in.
 */

import assert from 'node:assert/strict'
import type { ContractCase } from './cases'
import { apiGet, expectOk, waitUntil } from './helpers'
import { McpRequestError, McpSession, textOf } from './mcp-client'
import { startRustServer } from './rust-server'

const EXPECTED_LEGACY_TOOLS = [
  'act',
  'diff',
  'download',
  'evaluate',
  'grep',
  'history',
  'mark_skill_run',
  'name_session',
  'navigate',
  'pdf',
  'playwright',
  'read',
  'run',
  'save_skill',
  'screenshot',
  'snapshot',
  'tab_groups',
  'tabs',
  'upload',
  'wait',
  'windows',
]

const DEFAULT_TOOLS = [
  'playwright',
  'name_session',
  'save_skill',
  'mark_skill_run',
]

export const transportCases: ContractCase[] = [
  {
    name: 'transport: legacyTools exposes the full catalog including run and playwright',
    smoke: true,
    async run(ctx) {
      const tools = await ctx.mcp.listTools()
      const names = tools.map((tool) => tool.name).sort()
      if (!Bun.deepEquals(names, EXPECTED_LEGACY_TOOLS)) {
        throw new Error(`unexpected tool catalog: ${names.join(', ')}`)
      }
    },
  },
  {
    name: 'transport: default surface exposes only Playwright and session/tasks tools',
    smoke: true,
    async run(ctx) {
      // Use a separate sidecar with legacyTools omitted. The main server keeps
      // its compatibility opt-in for the older behavioral contracts.
      const server = await startRustServer(ctx.browser.cdpPort, false)
      let session: McpSession | undefined
      try {
        session = await McpSession.connect(
          server.baseUrl,
          'default-surface-contract',
        )
        assert.deepEqual(
          (await session.listTools()).map((tool) => tool.name),
          DEFAULT_TOOLS,
        )
        const hidden = EXPECTED_LEGACY_TOOLS.filter(
          (name) => !DEFAULT_TOOLS.includes(name),
        )
        let unknown: McpRequestError | undefined
        try {
          await session.callTool('unknown-surface-tool')
          assert.fail('unknown tool unexpectedly accepted')
        } catch (error) {
          assert.ok(error instanceof McpRequestError, String(error))
          assert.equal(error.code, -32601)
          unknown = error
        }
        for (const name of hidden) {
          await assert.rejects(session.callTool(name), (error) => {
            assert.ok(error instanceof McpRequestError, `${name}: ${error}`)
            assert.equal(error.code, unknown?.code)
            assert.equal(error.message, unknown?.message)
            assert.deepEqual(error.data, unknown?.data)
            return true
          })
        }
        expectOk(
          await session.callTool('name_session', { name: 'surface check' }),
          'name default session',
        )
        const active = session
        await waitUntil(
          async () => {
            const result = await active.callTool('playwright', {
              code: 'return true;',
            })
            if (
              result.isError &&
              textOf(result).includes('browser session not connected')
            )
              return false
            expectOk(result, 'default playwright readiness')
            return true
          },
          'default server to attach',
          { timeoutMs: 30000, intervalMs: 100 },
        )
        const result = await session.callTool('playwright', {
          code: `const p = await context.newPage();
await p.goto(${JSON.stringify(ctx.fixture('/links.html'))});
await expect(p.getByRole('heading', {name:'Links fixture', exact:true})).toBeVisible();
return {read:(await neo.read(p)).includes('Plain paragraph text'), pages:context.pages().length};`,
        })
        expectOk(result, 'default Playwright and internal read delegation')
        assert.deepEqual(result.structuredContent?.value, {
          read: true,
          pages: 1,
        })
        // Claiming a script-created page requires the full internal catalog's
        // tab_groups handler even when that handler cannot be called over MCP.
        await waitUntil(async () => {
          const response = await apiGet(server, '/api/v1/sessions?status=live')
          assert.equal(response.status, 200)
          const body = (await response.json()) as {
            items: Array<{
              sessionId: string
              live?: { browserTabs: unknown[] }
            }>
          }
          return (
            body.items.find((item) => item.sessionId === active.sessionId)?.live
              ?.browserTabs.length === 1
          )
        }, 'default session to own its Playwright page')
      } finally {
        try {
          if (session) {
            try {
              expectOk(
                await session.callTool('playwright', {
                  code: 'for (const p of await context.pages()) await p.close();',
                }),
                'default page cleanup',
              )
            } finally {
              await session.close()
            }
          }
        } finally {
          await server.stop()
        }
      }
    },
  },
  {
    name: 'transport: browser-shaped requests are rejected with 403',
    smoke: true,
    async run(ctx) {
      const body = JSON.stringify({
        jsonrpc: '2.0',
        id: 99,
        method: 'tools/list',
        params: {},
      })
      const withOrigin = await fetch(`${ctx.server.baseUrl}/mcp`, {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
          origin: 'http://evil.example',
        },
        body,
      })
      const withSecFetch = await fetch(`${ctx.server.baseUrl}/mcp`, {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
          'sec-fetch-site': 'cross-site',
        },
        body,
      })
      if (withOrigin.status !== 403 || withSecFetch.status !== 403) {
        throw new Error(
          `expected 403 for browser-shaped requests, got ${withOrigin.status}/${withSecFetch.status}`,
        )
      }
    },
  },
  {
    name: 'transport: unknown mcp-session-id is rejected',
    async run(ctx) {
      const response = await fetch(`${ctx.server.baseUrl}/mcp`, {
        method: 'POST',
        headers: {
          'content-type': 'application/json',
          accept: 'application/json, text/event-stream',
          'mcp-session-id': 'bogus-session-id-123',
        },
        body: JSON.stringify({
          jsonrpc: '2.0',
          id: 1,
          method: 'tools/list',
          params: {},
        }),
      })
      if (response.status < 400) {
        throw new Error(
          `expected an unknown session id to be rejected, got ${response.status}`,
        )
      }
    },
  },
  {
    name: 'transport: DELETE /mcp ends the session and audit records it',
    async run(ctx) {
      // Bodyless teardown does not require a JSON content type.
      const probe = await ctx.openSession('claw-contract-bare-delete')
      const bare = await fetch(`${ctx.server.baseUrl}/mcp`, {
        method: 'DELETE',
        headers: probe.sessionId ? { 'mcp-session-id': probe.sessionId } : {},
      })
      if (bare.status >= 400) {
        throw new Error(`bare DELETE /mcp failed with ${bare.status}`)
      }

      const session = await ctx.openSession('claw-contract-teardown')
      await ctx.openPage(ctx.fixture('/links.html'), session)
      const sessionId = session.sessionId
      if (!sessionId) throw new Error('session id missing after initialize')

      const teardown = await session.close()
      if (teardown.status >= 400) {
        throw new Error(`DELETE /mcp failed with ${teardown.status}`)
      }

      await waitUntil(
        async () => {
          const detail = await apiGet(
            ctx.server,
            `/api/v1/sessions/${sessionId}`,
          )
          if (!detail.ok) return false
          const payload = (await detail.json()) as {
            session?: { status?: string }
            summary?: { status?: string }
          }
          const status = payload.session?.status ?? payload.summary?.status
          return status !== undefined && status !== 'live'
        },
        'audit to record the session end',
        { timeoutMs: 20_000 },
      )
    },
  },
]
