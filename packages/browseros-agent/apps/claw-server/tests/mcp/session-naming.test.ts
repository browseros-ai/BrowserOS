import { describe, expect, it } from 'bun:test'
import {
  type ClientCapabilities,
  type ElicitRequestFormParams,
  type ElicitResult,
  ErrorCode,
  McpError,
} from '@modelcontextprotocol/sdk/types.js'
import {
  agentIdentityFromClient,
  createIdentityService,
} from '../../src/lib/mcp-session'
import {
  type RequestSessionNamingDeps,
  requestSessionNaming,
  type SessionNamingServer,
} from '../../src/mcp/session-naming'

interface ElicitCall {
  params: ElicitRequestFormParams
  options: { timeout: number }
}

function fakeServer(input: {
  capabilities?: ClientCapabilities
  results?: Array<ElicitResult | Error>
}): SessionNamingServer & { calls: ElicitCall[] } {
  const calls: ElicitCall[] = []
  const results = [...(input.results ?? [])]
  return {
    calls,
    getClientCapabilities: () => input.capabilities,
    elicitInput: async (params, options) => {
      calls.push({ params, options })
      const result = results.shift()
      if (result instanceof Error) throw result
      if (!result) throw new Error('no fake result queued')
      return result
    },
  }
}

function setup() {
  const identityService = createIdentityService({ now: () => 1_000 })
  const identity = identityService.registerInitialize({
    sessionId: 'sid-1',
    clientInfo: { name: 'Claude Code', version: '1.0.0' },
  })
  const applyCalls: Array<{
    agentId: string
    title: string
    session: unknown
  }> = []
  const delays: number[] = []
  const deps: RequestSessionNamingDeps = {
    identityService,
    getBrowserSession: () => ({ fake: true }) as never,
    applyTitle: async (input) => {
      applyCalls.push(input)
    },
    delay: async (ms) => {
      delays.push(ms)
    },
  }
  return { applyCalls, delays, deps, identity, identityService }
}

describe('requestSessionNaming', () => {
  it('does not elicit when the client lacks elicitation capability', async () => {
    const { deps } = setup()
    const server = fakeServer({ capabilities: {} })
    await requestSessionNaming({ server, sessionId: 'sid-1' }, deps)
    expect(server.calls).toEqual([])
  })

  it('stores accepted names and applies the tab-group title', async () => {
    const { applyCalls, deps, identity, identityService } = setup()
    const server = fakeServer({
      capabilities: { elicitation: {} },
      results: [
        {
          action: 'accept',
          content: { name: 'Invoice Processing' },
        },
      ],
    })

    await requestSessionNaming({ server, sessionId: 'sid-1' }, deps)

    expect(identityService.getIdentity('sid-1')?.sessionLabel).toBe(
      'invoice-processing',
    )
    expect(applyCalls).toEqual([
      {
        agentId: agentIdentityFromClient(identity).agentId,
        title: 'claude/invoice-processing',
        session: { fake: true },
      },
    ])
    expect(server.calls[0]?.params.message).toContain(
      'Tabs will be grouped as claude/<name>',
    )
    expect(server.calls[0]?.params.requestedSchema.required).toEqual(['name'])
  })

  it('ignores accepted names that normalize to empty', async () => {
    const { applyCalls, deps, identityService } = setup()
    const server = fakeServer({
      capabilities: { elicitation: {} },
      results: [{ action: 'accept', content: { name: '!!!' } }],
    })
    await requestSessionNaming({ server, sessionId: 'sid-1' }, deps)
    expect(identityService.getIdentity('sid-1')?.sessionLabel).toBeNull()
    expect(applyCalls).toEqual([])
  })

  it('ignores decline and cancel results', async () => {
    for (const action of ['decline', 'cancel'] as const) {
      const { applyCalls, deps, identityService } = setup()
      const server = fakeServer({
        capabilities: { elicitation: {} },
        results: [{ action }],
      })
      await requestSessionNaming({ server, sessionId: 'sid-1' }, deps)
      expect(identityService.getIdentity('sid-1')?.sessionLabel).toBeNull()
      expect(applyCalls).toEqual([])
    }
  })

  it('resolves after two elicitation failures without applying a title', async () => {
    const { applyCalls, delays, deps, identityService } = setup()
    const server = fakeServer({
      capabilities: { elicitation: {} },
      results: [new Error('no stream yet'), new Error('still no stream')],
    })
    await requestSessionNaming({ server, sessionId: 'sid-1' }, deps)
    expect(server.calls).toHaveLength(2)
    expect(delays).toEqual([2_000])
    expect(identityService.getIdentity('sid-1')?.sessionLabel).toBeNull()
    expect(applyCalls).toEqual([])
  })

  it('does not retry when the user ignores the elicitation prompt', async () => {
    const { applyCalls, delays, deps, identityService } = setup()
    const server = fakeServer({
      capabilities: { elicitation: {} },
      results: [new McpError(ErrorCode.RequestTimeout, 'timeout')],
    })
    await requestSessionNaming({ server, sessionId: 'sid-1' }, deps)
    expect(server.calls).toHaveLength(1)
    expect(delays).toEqual([])
    expect(identityService.getIdentity('sid-1')?.sessionLabel).toBeNull()
    expect(applyCalls).toEqual([])
  })
})
