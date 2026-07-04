import { describe, expect, it } from 'bun:test'
import type { BrowserSession } from '@browseros/browser-core/core/session'
import { registerBrowserTools } from '@browseros/browser-mcp/register'
import { BROWSER_TOOLS } from '@browseros/browser-mcp/registry'

type RegisteredHandler = (
  args: Record<string, unknown>,
  extra?: { signal?: AbortSignal },
) => Promise<{
  content: unknown
  isError?: boolean
  structuredContent?: unknown
}>

function createFakeServer() {
  const handlers = new Map<string, RegisteredHandler>()
  const configs = new Map<
    string,
    { description: string; inputSchema?: unknown; annotations?: unknown }
  >()

  return {
    handlers,
    configs,
    server: {
      registerTool(
        name: string,
        config: {
          description: string
          inputSchema?: unknown
          annotations?: unknown
        },
        handler: RegisteredHandler,
      ) {
        configs.set(name, config)
        handlers.set(name, handler)
      },
    },
  }
}

function textOf(result: Awaited<ReturnType<RegisteredHandler>> | undefined) {
  if (!Array.isArray(result?.content)) return ''
  return result.content
    .filter(
      (item): item is { type: 'text'; text: string } =>
        typeof item === 'object' &&
        item !== null &&
        'type' in item &&
        item.type === 'text' &&
        'text' in item &&
        typeof item.text === 'string',
    )
    .map((item) => item.text)
    .join('\n')
}

describe('registerBrowserTools', () => {
  it('registers the shared browser tool surface', () => {
    const fake = createFakeServer()

    registerBrowserTools(
      fake.server as never,
      { pages: {} } as unknown as BrowserSession,
    )

    expect([...fake.handlers.keys()]).toEqual(
      BROWSER_TOOLS.map((tool) => tool.name),
    )
    expect(fake.configs.get('tabs')?.inputSchema).toBeDefined()
    expect(fake.configs.get('snapshot')?.annotations).toEqual({
      readOnlyHint: true,
    })
  })

  it('logs sampled registration and records failed tool executions', async () => {
    const fake = createFakeServer()
    const debugLogs: Array<{
      message: string
      meta?: Record<string, unknown>
    }> = []
    const infoLogs: Array<{ message: string; meta?: Record<string, unknown> }> =
      []
    const events: Array<Record<string, unknown>> = []
    const session = {
      pages: {
        newPage: async () => {
          throw new Error('tab creation failed')
        },
      },
    } as unknown as BrowserSession

    registerBrowserTools(
      fake.server as never,
      session,
      {},
      {
        logger: {
          debug: (message, meta) => debugLogs.push({ message, meta }),
          info: (message, meta) => infoLogs.push({ message, meta }),
        },
        onToolExecuted: (event) => events.push(event),
        shouldLogToolRegistration: () => true,
        source: 'unit-test',
      },
    )

    const result = await fake.handlers.get('tabs')?.({
      action: 'new',
      url: 'https://example.com',
    })

    expect(infoLogs).toEqual([
      {
        message: 'Registered browser MCP tools',
        meta: expect.objectContaining({
          count: BROWSER_TOOLS.length,
          source: 'unit-test',
        }),
      },
      {
        message: 'MCP browser tool returned error',
        meta: expect.objectContaining({
          toolName: 'tabs',
          source: 'unit-test',
          errorSummary: expect.objectContaining({
            contentCount: expect.any(Number),
            textBlockCount: expect.any(Number),
            textLength: expect.any(Number),
            lineCount: expect.any(Number),
          }),
        }),
      },
    ])
    expect(JSON.stringify(infoLogs)).not.toContain('tab creation failed')
    expect(debugLogs.map((log) => log.message)).toEqual([
      'MCP browser tool started',
      'MCP browser tool completed',
    ])
    expect(debugLogs[0]?.meta).toEqual(
      expect.objectContaining({
        toolName: 'tabs',
        source: 'unit-test',
        args: expect.objectContaining({
          action: 'new',
          urlOrigin: 'https://example.com',
        }),
      }),
    )
    expect(result?.isError).toBe(true)
    expect(textOf(result)).toContain('tab creation failed')
    expect(events).toEqual([
      expect.objectContaining({
        tool_name: 'tabs',
        success: false,
        source: 'unit-test',
      }),
    ])
    expect(events[0]?.duration_ms).toEqual(expect.any(Number))
  })

  it('fires lifecycle callbacks around browser tool execution', async () => {
    const fake = createFakeServer()
    let resolveNewPage: ((value: number) => void) | undefined
    const starts: Array<Record<string, unknown>> = []
    const ends: Array<Record<string, unknown>> = []
    const session = {
      pages: {
        newPage: () =>
          new Promise<number>((resolve) => {
            resolveNewPage = resolve
          }),
      },
    } as unknown as BrowserSession

    registerBrowserTools(
      fake.server as never,
      session,
      {},
      {
        onToolExecutionStart: (event) => starts.push(event),
        onToolExecutionEnd: (event) => ends.push(event),
        source: 'unit-test',
      },
    )

    const run = fake.handlers.get('tabs')?.({
      action: 'new',
      url: 'https://example.com',
    })
    await Promise.resolve()

    expect(starts).toEqual([{ tool_name: 'tabs', source: 'unit-test' }])
    expect(ends).toEqual([])

    resolveNewPage?.(42)
    await run

    expect(ends).toEqual([{ tool_name: 'tabs', source: 'unit-test' }])
  })
})
