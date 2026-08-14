import { describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { BROWSER_MCP_INSTRUCTIONS } from '@browseros/browser-mcp/mcp-prompt'
import { createBrowserMcpServer } from '@browseros/browser-mcp/mcp-server'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'

type RegisteredTool = {
  description: string
  handler: (
    args: Record<string, unknown>,
    extra?: { mcpReq?: { signal?: AbortSignal } },
  ) => Promise<{
    content: unknown
    isError?: boolean
    structuredContent?: unknown
  }>
}

type InspectableBrowserMcpServer = {
  _registeredTools: Record<string, RegisteredTool>
  server: {
    _capabilities: Record<string, unknown>
    _instructions?: string
    _requestHandlers: Map<
      string,
      (
        request: Record<string, unknown>,
        extra: Record<string, unknown>,
      ) => Promise<unknown> | unknown
    >
  }
}

function inspect(server: unknown) {
  return server as InspectableBrowserMcpServer
}

describe('createBrowserMcpServer', () => {
  it('creates a browser-only MCP server with the shared tool catalogue', () => {
    const server = inspect(
      createBrowserMcpServer({
        name: 'browseros_mcp',
        title: 'BrowserOS MCP server',
        version: '1.2.3',
        browserSession: { pages: {} } as unknown as BrowserSession,
      }),
    )

    expect(Object.keys(server._registeredTools)).toEqual(
      BROWSER_TOOLS.map((tool) => tool.name),
    )
    expect(server.server._capabilities).toEqual({
      tools: { listChanged: true },
    })
    expect(server.server._instructions).toBe(BROWSER_MCP_INSTRUCTIONS)
    expect(server.server._requestHandlers.has('logging/setLevel')).toBe(false)
  })

  it('exposes a real JSON input schema for tools/list', () => {
    const server = createBrowserMcpServer({
      name: 'browseros_mcp',
      title: 'BrowserOS MCP server',
      version: '1.2.3',
      browserSession: { pages: {} } as unknown as BrowserSession,
    }) as unknown as {
      toolInputSchemaJson(name: string): Record<string, unknown> | undefined
    }

    const schema = server.toolInputSchemaJson('tabs') as {
      type?: string
      properties?: Record<string, unknown>
    }

    expect(schema?.type).toBe('object')
    expect(Object.keys(schema?.properties ?? {}).sort()).toEqual([
      'action',
      'background',
      'page',
      'url',
    ])
  })

  it('passes defaults and registration hooks through to browser tools', async () => {
    const calls: Array<{
      url: string
      opts?: {
        background?: boolean
        windowId?: number
        tabGroupId?: string
      }
    }> = []
    const events: Array<Record<string, unknown>> = []
    const server = inspect(
      createBrowserMcpServer({
        name: 'browseros_mcp',
        title: 'BrowserOS MCP server',
        version: '1.2.3',
        browserSession: {
          pages: {
            newPage: async (
              url: string,
              opts?: {
                background?: boolean
                windowId?: number
                tabGroupId?: string
              },
            ) => {
              calls.push({ url, opts })
              return 42
            },
          },
        } as unknown as BrowserSession,
        defaultWindowId: 7,
        defaultTabGroupId: 'group-a',
        instructions: 'custom browser instructions',
        registration: {
          source: 'unit-test',
          onToolExecuted: (event) => events.push(event),
        },
      }),
    )

    const result = await server._registeredTools.tabs.handler({
      action: 'new',
      url: 'https://example.com',
    })

    expect(server.server._instructions).toBe('custom browser instructions')
    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({ page: 42 })
    expect(calls).toEqual([
      {
        url: 'https://example.com',
        opts: {
          background: true,
          windowId: 7,
          tabGroupId: 'group-a',
        },
      },
    ])
    expect(events).toEqual([
      expect.objectContaining({
        tool_name: 'tabs',
        success: true,
        source: 'unit-test',
      }),
    ])
    expect(events[0]?.duration_ms).toEqual(expect.any(Number))
  })

  it('returns the focused page through the tabs active action', async () => {
    const activePage = {
      pageId: 42,
      targetId: 'target-42',
      tabId: 9,
      url: 'https://example.com',
      title: 'Example',
      isActive: true,
      isLoading: false,
      loadProgress: 1,
      isPinned: false,
    }
    const server = inspect(
      createBrowserMcpServer({
        name: 'browseros_mcp',
        title: 'BrowserOS MCP server',
        version: '1.2.3',
        browserSession: {
          pages: {
            getActive: async () => activePage,
          },
        } as unknown as BrowserSession,
      }),
    )

    const result = await server._registeredTools.tabs.handler({
      action: 'active',
    })

    expect(result?.isError).toBeFalsy()
    expect(result?.structuredContent).toEqual({
      action: 'active',
      page: activePage,
    })
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: expect.stringContaining('[42] https://example.com (Example)'),
      }),
    ])
  })

  it('returns an error when no focused page exists', async () => {
    const server = inspect(
      createBrowserMcpServer({
        name: 'browseros_mcp',
        title: 'BrowserOS MCP server',
        version: '1.2.3',
        browserSession: {
          pages: {
            getActive: async () => null,
          },
        } as unknown as BrowserSession,
      }),
    )

    const result = await server._registeredTools.tabs.handler({
      action: 'active',
    })

    expect(result?.isError).toBe(true)
    expect(result?.content).toEqual([
      expect.objectContaining({
        type: 'text',
        text: 'tabs active: no active page found.',
      }),
    ])
  })

  it('mints, echoes, and strips a session handle when sessionIdentity is on', async () => {
    const debugLogs: Array<{ msg: string; meta?: Record<string, unknown> }> = []
    const raw = createBrowserMcpServer({
      name: 'browseros_mcp',
      title: 'BrowserOS MCP server',
      version: '1.2.3',
      browserSession: {
        pages: {
          newPage: async () => 42,
        },
      } as unknown as BrowserSession,
      registration: {
        sessionIdentity: true,
        logger: {
          debug: (msg: string, meta?: Record<string, unknown>) =>
            debugLogs.push({ msg, meta }),
        },
      },
    })
    const server = inspect(raw)

    const schema = (
      raw as unknown as {
        toolInputSchemaJson(name: string): {
          properties?: Record<string, unknown>
        }
      }
    ).toolInputSchemaJson('tabs')
    expect(Object.keys(schema?.properties ?? {})).toContain('session')

    // No handle provided: the server mints one and returns it, and the tool
    // still runs (the session key is stripped before the tool parses args).
    const minted = await server._registeredTools.tabs.handler({
      action: 'new',
      url: 'https://example.com',
    })
    const mintedSession = (minted?.structuredContent as { session?: string })
      ?.session
    expect(typeof mintedSession).toBe('string')
    expect(mintedSession).toHaveLength(36)
    // The handle is attributed on the per-call log, not the aggregated metric.
    const started = debugLogs.find(
      (entry) => entry.msg === 'MCP browser tool started',
    )
    expect(started?.meta?.session).toBe(mintedSession)

    // A provided handle is echoed back unchanged.
    const reused = await server._registeredTools.tabs.handler({
      action: 'new',
      url: 'https://example.com',
      session: 'agent-supplied-handle',
    })
    expect((reused?.structuredContent as { session?: string })?.session).toBe(
      'agent-supplied-handle',
    )
  })

  it('threads the abort signal from extra.mcpReq.signal into the tool', async () => {
    const server = inspect(
      createBrowserMcpServer({
        name: 'browseros_mcp',
        title: 'BrowserOS MCP server',
        version: '1.2.3',
        browserSession: { pages: {} } as unknown as BrowserSession,
      }),
    )

    // The signal is delivered where the v2 SDK actually puts it (mcpReq.signal),
    // so `wait` aborts immediately instead of pausing. The previous top-level
    // `extra.signal` read was always undefined and never reached the tool.
    const result = await server._registeredTools.wait.handler(
      { page: 1, for: 'time', value: 60000 },
      { mcpReq: { signal: AbortSignal.abort() } },
    )

    expect(result?.isError).toBe(true)
  })
})
