import { describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { BROWSER_MCP_INSTRUCTIONS } from '@browseros/browser-mcp/mcp-prompt'
import { createBrowserMcpServer } from '@browseros/browser-mcp/mcp-server'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'

type RegisteredTool = {
  description: string
  handler: (
    args: Record<string, unknown>,
    extra?: { signal?: AbortSignal },
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
      logging: {},
      tools: { listChanged: true },
    })
    expect(server.server._instructions).toBe(BROWSER_MCP_INSTRUCTIONS)
    expect(server.server._requestHandlers.has('logging/setLevel')).toBe(true)
  })

  it('passes defaults and registration hooks through to browser tools', async () => {
    const calls: Array<{
      url: string
      opts?: {
        background?: boolean
        hidden?: boolean
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
                hidden?: boolean
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
          hidden: false,
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
      isHidden: false,
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
})
