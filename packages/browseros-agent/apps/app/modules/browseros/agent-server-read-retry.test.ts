import { expect, test } from 'bun:test'
import { ServerApiClient } from '@browseros/server-api/client'
import { QueryClient } from '@tanstack/react-query'
import { agentServerReadRetry } from './agent-server-read-retry'

const startupReads = [
  [
    'providers',
    (client: ServerApiClient) => client.listProviders(),
    { providers: [] },
  ],
  [
    'default provider',
    (client: ServerApiClient) => client.getDefaultProvider(),
    { provider: null },
  ],
  ['agents', (client: ServerApiClient) => client.listAgents(), { agents: [] }],
] as const

test.each(startupReads)(
  '%s keeps asking until the server answers',
  async (name, read, body) => {
    let requests = 0
    const api = new ServerApiClient('http://127.0.0.1:1', {
      fetch: async () => {
        if (++requests <= 5) throw new TypeError('Failed to fetch')
        return Response.json(body)
      },
    })
    const queries = new QueryClient()

    await expect(
      queries.fetchQuery<unknown>({
        ...agentServerReadRetry,
        queryKey: [name],
        queryFn: () => read(api),
        retryDelay: 0,
      }),
    ).resolves.toEqual(body)
    expect(requests).toBe(6)
  },
)

test.each([400, 503])(
  'an HTTP %i keeps the normal retry limit',
  async (status) => {
    let requests = 0
    const api = new ServerApiClient('http://127.0.0.1:1', {
      fetch: async () => {
        requests++
        return Response.json({ error: 'test error' }, { status })
      },
    })
    const queries = new QueryClient()

    await expect(
      queries.fetchQuery({
        ...agentServerReadRetry,
        queryKey: ['providers'],
        queryFn: () => api.listProviders(),
        retryDelay: 0,
      }),
    ).rejects.toMatchObject({ status, apiMessage: 'test error' })
    expect(requests).toBe(4)
  },
)
