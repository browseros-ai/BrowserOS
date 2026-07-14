import { describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { createMcpServer } from '../../../../src/api/services/mcp/mcp-server'

type RegisteredTool = {
  handler: (args: Record<string, unknown>) => Promise<{
    content: unknown
    isError?: boolean
    structuredContent?: unknown
  }>
}

type InspectableMcpServer = {
  _registeredTools: Record<string, RegisteredTool>
}

function inspect(server: unknown): InspectableMcpServer {
  return server as InspectableMcpServer
}

function browserSession(): BrowserSession {
  return {
    pages: {
      newPage: async () => 42,
      getInfo: () => ({ url: 'https://example.com' }),
    },
    observe: () => ({
      snapshot: async () => ({ text: 'button "Save" [ref=e1]' }),
    }),
  } as unknown as BrowserSession
}

describe('createMcpServer structured browser results', () => {
  it('strips ordinary envelopes by default while retaining run output', async () => {
    const server = inspect(
      createMcpServer({
        version: 'test',
        browserSession: browserSession(),
        executionDir: '/tmp/browseros-execution',
      }),
    )

    const tabs = await server._registeredTools.tabs.handler({ action: 'new' })
    const run = await server._registeredTools.run.handler({ code: 'return 42' })

    expect(tabs).not.toHaveProperty('structuredContent')
    expect(run.structuredContent).toEqual({ ok: true, value: 42, logs: [] })
  })

  it('returns grep matches for opted-in machine clients', async () => {
    const server = inspect(
      createMcpServer({
        version: 'test',
        browserSession: browserSession(),
        executionDir: '/tmp/browseros-execution',
        includeStructuredContent: true,
      }),
    )

    const result = await server._registeredTools.grep.handler({
      page: 1,
      pattern: 'save',
      over: 'ax',
    })

    expect(result.structuredContent).toEqual({
      page: 1,
      over: 'ax',
      count: 1,
      matches: ['button "Save" [ref=e1]'],
    })
  })
})
