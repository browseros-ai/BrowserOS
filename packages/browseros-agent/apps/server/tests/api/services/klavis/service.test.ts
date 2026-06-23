/**
 * @license
 * Copyright 2025 BrowserOS
 */

import { describe, expect, it, mock } from 'bun:test'
import type { KlavisClient } from '../../../../src/api/services/klavis/client'
import { KlavisService } from '../../../../src/api/services/klavis/service'
import type {
  KlavisSessionHandle,
  StrataCreateResponse,
  UserIntegration,
} from '../../../../src/api/services/klavis/types'

class StubKlavisClient {
  integrations: UserIntegration[] = []
  createStrataCalls: string[][] = []

  async createStrata(
    userId: string,
    servers: string[],
  ): Promise<StrataCreateResponse> {
    this.createStrataCalls.push(servers)
    return {
      strataServerUrl: `https://strata.test/${userId}`,
      strataId: 'strata-123',
      addedServers: servers,
      oauthUrls: Object.fromEntries(
        servers.map((server) => [server, `https://oauth.test/${server}`]),
      ),
      apiKeyUrls: {},
    }
  }

  async getUserIntegrations(): Promise<UserIntegration[]> {
    return this.integrations
  }

  submitApiKey = mock(async () => {})
  deleteServersFromStrata = mock(async () => {})
}

const asClient = (stub: StubKlavisClient): KlavisClient =>
  stub as unknown as KlavisClient

function createHandle(
  overrides: Partial<KlavisSessionHandle> = {},
): KlavisSessionHandle {
  return {
    browserosId: 'browseros-1',
    tools: [
      {
        name: 'gmail_search',
        description: 'Search Gmail',
        inputSchema: { type: 'object' },
      } as never,
    ],
    inputSchemas: new Map([['gmail_search', {} as never]]),
    callTool: mock(async () => ({
      content: [
        { type: 'text', text: 'Found 2 threads' },
        {
          type: 'image',
          data: 'ZmFrZS1pbWFnZQ==',
          mimeType: 'image/png',
        },
      ],
    })),
    close: mock(async () => {}),
    ...overrides,
  }
}

const nextTick = () => new Promise((resolve) => setTimeout(resolve, 0))

describe('KlavisService', () => {
  it('exposes connector discovery before the proxy is ready', async () => {
    const client = new StubKlavisClient()
    client.integrations = [{ name: 'Slack', isAuthenticated: true }]
    const service = new KlavisService({
      browserosId: 'browseros-1',
      client: asClient(client),
      connect: async () => await new Promise(() => {}),
    })

    service.start()

    const toolSet = service.buildAiSdkToolSet({
      selectedServerNames: ['Slack'],
    })
    expect(toolSet.connector_mcp_servers).toBeDefined()
    expect(toolSet.gmail_search).toBeUndefined()

    const inventory = await toolSet.connector_mcp_servers.execute?.({})
    expect(inventory).toMatchObject({
      selected: ['Slack'],
      connected: [{ name: 'Slack', isAuthenticated: true }],
      proxy: { state: 'connecting' },
    })
  })

  it('maps MCP content results into model content parts when ready', async () => {
    const handle = createHandle()
    const service = new KlavisService({
      browserosId: 'browseros-1',
      client: asClient(new StubKlavisClient()),
      connect: async () => handle,
    })

    service.start()
    await nextTick()

    const toolSet = service.buildAiSdkToolSet()
    const searchTool = toolSet.gmail_search

    expect(searchTool).toBeDefined()

    const output = await searchTool.execute?.({})
    const modelOutput = await searchTool.toModelOutput?.({
      toolCallId: 'call-1',
      input: {},
      output,
    })

    expect(modelOutput).toEqual({
      type: 'content',
      value: [
        { type: 'text', text: 'Found 2 threads' },
        {
          type: 'image-data',
          data: 'ZmFrZS1pbWFnZQ==',
          mediaType: 'image/png',
        },
      ],
    })
  })

  it('maps null MCP results into JSON model output', async () => {
    const handle = createHandle({
      tools: [
        {
          name: 'notion_lookup',
          description: 'Lookup Notion',
          inputSchema: { type: 'object' },
        } as never,
      ],
      inputSchemas: new Map([['notion_lookup', {} as never]]),
      callTool: mock(async () => null as never),
    })
    const service = new KlavisService({
      browserosId: 'browseros-1',
      client: asClient(new StubKlavisClient()),
      connect: async () => handle,
    })

    service.start()
    await nextTick()

    const toolSet = service.buildAiSdkToolSet()
    const lookupTool = toolSet.notion_lookup
    const output = await lookupTool.execute?.({})
    const modelOutput = await lookupTool.toModelOutput?.({
      toolCallId: 'call-2',
      input: {},
      output,
    })

    expect(modelOutput).toEqual({
      type: 'json',
      value: null,
    })
  })

  it('returns an auth URL for an unconnected requested connector', async () => {
    const client = new StubKlavisClient()
    const service = new KlavisService({
      browserosId: 'browseros-1',
      client: asClient(client),
      retryDelaysMs: [],
      connect: async () => {
        throw new Error('offline')
      },
    })

    service.start()
    await nextTick()

    const toolSet = service.buildAiSdkToolSet()
    const payload = await toolSet.connector_mcp_servers.execute?.({
      server_name: 'Slack',
    })

    expect(payload).toMatchObject({
      connected: false,
      server_name: 'Slack',
      authUrl: 'https://oauth.test/Slack',
      proxy: { state: 'unavailable', error: 'offline' },
    })
    expect(client.createStrataCalls).toEqual([['Slack']])
  })

  it('retries in the background until a connection succeeds', async () => {
    const handle = createHandle({ tools: [] })
    let attempts = 0
    const service = new KlavisService({
      browserosId: 'browseros-1',
      client: asClient(new StubKlavisClient()),
      retryDelaysMs: [1],
      connect: async () => {
        attempts++
        if (attempts === 1) {
          throw new Error('boom')
        }
        return handle
      },
    })

    service.start()
    await new Promise((resolve) => setTimeout(resolve, 20))

    expect(attempts).toBe(2)
    expect(service.getProxyStatus()).toEqual({ state: 'ready', toolCount: 0 })
    await service.stop()
  })

  it('closes a late handle when stopped during an in-flight connect', async () => {
    let releaseConnect: (() => void) | undefined
    const connectStarted = new Promise<void>((resolve) => {
      releaseConnect = resolve
    })
    const handle = createHandle({ tools: [] })
    const service = new KlavisService({
      browserosId: 'browseros-2',
      client: asClient(new StubKlavisClient()),
      connect: async () => {
        await connectStarted
        return handle
      },
    })

    service.start()
    await service.stop()
    releaseConnect?.()
    await nextTick()

    expect(handle.close).toHaveBeenCalledTimes(1)
    expect(service.getProxyStatus()).toEqual({ state: 'stopped' })
  })

  it('cancels a scheduled retry when stopped', async () => {
    let resolveFirstAttempt: (() => void) | undefined
    const firstAttemptDone = new Promise<void>((resolve) => {
      resolveFirstAttempt = resolve
    })
    let attempts = 0
    const service = new KlavisService({
      browserosId: 'browseros-3',
      client: asClient(new StubKlavisClient()),
      retryDelaysMs: [20],
      connect: async () => {
        attempts++
        if (attempts === 1) {
          resolveFirstAttempt?.()
          throw new Error('boom')
        }
        return createHandle({ tools: [] })
      },
    })

    service.start()
    await firstAttemptDone
    await service.stop()
    await new Promise((resolve) => setTimeout(resolve, 40))

    expect(attempts).toBe(1)
    expect(service.getProxyStatus()).toEqual({ state: 'stopped' })
  })
})
