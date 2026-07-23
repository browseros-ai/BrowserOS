/**
 * Transport & session-plumbing cases (cross-cutting invariant [7]):
 * the `/mcp` endpoint itself — catalog exposure, browser-request
 * hygiene, session-id handling, DELETE teardown + audit tie-in.
 */

import type { ContractCase } from './cases'
import { apiGet, waitUntil } from './helpers'

const EXPECTED_TOOLS = [
  'act',
  'diff',
  'download',
  'evaluate',
  'grep',
  'name_session',
  'navigate',
  'pdf',
  'read',
  'run',
  'screenshot',
  'snapshot',
  'tab_groups',
  'tabs',
  'upload',
  'wait',
  'windows',
]

export const transportCases: ContractCase[] = [
  {
    name: 'transport: tools/list exposes the full 17-tool catalog',
    smoke: true,
    async run(ctx) {
      const tools = await ctx.mcp.listTools()
      const names = tools.map((tool) => tool.name).sort()
      if (!Bun.deepEquals(names, EXPECTED_TOOLS)) {
        throw new Error(`unexpected tool catalog: ${names.join(', ')}`)
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
