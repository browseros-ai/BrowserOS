import { parseArgs } from 'node:util'
import type { CaptchaPage } from '../fixtures/catalog'
import { probes } from './probes'
import { stubSolveResult } from './solve-result'

type Scenario = 'contract' | 'no-tool' | 'false-positive'
export async function startStubMcp(options: {
  scenario: Scenario
  pages?: readonly CaptchaPage[]
}) {
  const pages =
    options.pages ??
    (await import('../fixtures/catalog').then((c) => [
      ...c.FIXTURE_PAGES,
      ...c.DEMO_PAGES,
    ]))
  const sessions = new Map<
    string,
    Map<number, { entry: CaptchaPage; solved: boolean; submitted: boolean }>
  >()
  let nextPage = 0
  const calls: Array<{ name: string; args: Record<string, unknown> }> = []
  const server = Bun.serve({
    hostname: '127.0.0.1',
    port: 0,
    async fetch(request) {
      if (request.method === 'DELETE') {
        sessions.delete(request.headers.get('mcp-session-id') ?? '')
        return new Response(null, { status: 204 })
      }
      if (request.method !== 'POST')
        return new Response('MCP POST required', { status: 405 })
      const rpc = (await request.json()) as {
        id?: number
        method: string
        params?: { name: string; arguments: Record<string, unknown> }
      }
      const response = (result: unknown, headers = {}) =>
        Response.json({ jsonrpc: '2.0', id: rpc.id, result }, { headers })
      const failure = (code: number) =>
        Response.json({
          jsonrpc: '2.0',
          id: rpc.id,
          error: { code, message: 'tool not found' },
        })
      if (rpc.method === 'initialize') {
        const id = crypto.randomUUID()
        sessions.set(id, new Map())
        return response(
          {
            protocolVersion: '2025-03-26',
            capabilities: { tools: {} },
            serverInfo: { name: 'captcha-stub', version: '1' },
          },
          { 'mcp-session-id': id },
        )
      }
      if (rpc.method === 'notifications/initialized')
        return new Response(null, { status: 202 })
      const session = sessions.get(request.headers.get('mcp-session-id') ?? '')
      if (!session) return new Response('session required', { status: 404 })
      if (rpc.method === 'tools/list')
        return response({
          tools: [
            'tabs',
            'evaluate',
            ...(options.scenario === 'no-tool' ? [] : ['solve_captcha']),
          ].map((name) => ({ name, inputSchema: { type: 'object' } })),
        })
      if (rpc.method !== 'tools/call' || !rpc.params) return failure(-32601)
      const { name, arguments: args } = rpc.params
      calls.push({ name, args })
      const result = (value: unknown) =>
        response({
          content: [{ type: 'text', text: JSON.stringify(value) }],
          structuredContent: value,
        })
      if (name === 'tabs') {
        if (args.action === 'new') {
          const url = new URL(String(args.url))
          const entry = pages.find(
            (p) =>
              (p.url && new URL(p.url).href === url.href) ||
              p.path === url.pathname,
          )
          if (!entry) return failure(-32602)
          const page = ++nextPage
          session.set(page, { entry, solved: false, submitted: false })
          return result({ page })
        }
        if (args.action === 'close') session.delete(Number(args.page))
        return result({})
      }
      if (name === 'solve_captcha' && options.scenario === 'no-tool')
        return failure(-32601)
      const state = session.get(Number(args.page))
      if (!state) return failure(-32602)
      if (name === 'solve_captcha') {
        state.solved = true
        return response(
          stubSolveResult(
            options.scenario === 'false-positive'
              ? 'solved'
              : (state.entry.expect[0] ?? 'error'),
            state.entry.family,
          ),
        )
      }
      if (name !== 'evaluate') return failure(-32601)
      const code = String(args.code)
      const accepted =
        state.solved &&
        options.scenario !== 'false-positive' &&
        ['solved', 'already_clear'].includes(state.entry.expect[0] ?? '')
      let value: unknown
      if (code.startsWith(probes.ready)) value = state.entry.ready !== 'false'
      else if (code.startsWith(probes.observation))
        value = { tokenLength: accepted ? 123 : 0, callbackFired: accepted }
      else if (code.startsWith(probes.submit)) {
        state.submitted = true
        value = true
      } else if (code.startsWith(probes.body))
        value = state.submitted
          ? accepted
            ? 'ACCEPTED'
            : 'REJECTED: invalid-token'
          : ''
      else return failure(-32602)
      return response({
        content: [
          {
            type: 'text',
            text: `[UNTRUSTED_PAGE_CONTENT nonce=stub]\n${JSON.stringify(value)}\n[END_UNTRUSTED_PAGE_CONTENT nonce=stub]`,
          },
        ],
      })
    },
  })
  return {
    calls,
    url: `http://127.0.0.1:${server.port}`,
    stop: async () => {
      await server.stop(true)
    },
  }
}

if (import.meta.main) {
  const { values } = parseArgs({
    args: Bun.argv.slice(2),
    options: { scenario: { type: 'string', default: 'contract' } },
  })
  if (!['contract', 'no-tool', 'false-positive'].includes(values.scenario))
    throw new Error('unknown scenario')
  const stub = await startStubMcp({ scenario: values.scenario as Scenario })
  console.log(stub.url)
  for (const signal of ['SIGINT', 'SIGTERM'] as const)
    process.once(signal, () => {
      void stub.stop().then(() => process.exit(0))
    })
}
