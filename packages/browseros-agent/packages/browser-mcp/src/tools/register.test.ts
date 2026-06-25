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
    const infoMessages: string[] = []
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
        logger: { info: (message) => infoMessages.push(message) },
        onToolExecuted: (event) => events.push(event),
        shouldLogToolRegistration: () => true,
        source: 'unit-test',
      },
    )

    const result = await fake.handlers.get('tabs')?.({
      action: 'new',
      url: 'https://example.com',
    })

    expect(infoMessages).toEqual([
      expect.stringContaining(
        `Registered ${BROWSER_TOOLS.length} browser tools`,
      ),
    ])
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
})
