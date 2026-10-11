import { afterAll, describe, expect, it, mock } from 'bun:test'
import type { AgentSettings } from '@browseros/claw-api'
import { MutationObserver, QueryClient } from '@tanstack/react-query'
import * as api from './client'

const on: AgentSettings = { humanHelpEnabled: true }
const off: AgentSettings = { humanHelpEnabled: false }
const read = Promise.withResolvers<AgentSettings>()
const readStarted = Promise.withResolvers<void>()

mock.module('./client', () => ({
  ...api,
  apiClient: async () => ({
    getAgentSettings: async () => {
      readStarted.resolve()
      return read.promise
    },
    updateAgentSettings: async () => off,
  }),
}))

const { useAgentSettings, useUpdateAgentSettings } = await import(
  './agent-settings.hooks'
)

afterAll(() => mock.restore())

describe('agent settings save ordering', () => {
  it('tests that a save survives an older settings read that answers after it', async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    client.setQueryData(useAgentSettings.getKey(), on)
    // A refetch that read the old value before the save but answers after it.
    // The transport ignores AbortSignal, so cancelling must also drop its answer.
    const pendingRead = client
      .fetchQuery({ ...useAgentSettings.getOptions(), staleTime: 0 })
      .catch(() => undefined)
    await readStarted.promise
    const mutation = new MutationObserver(
      client,
      useUpdateAgentSettings.getOptions(),
    )
    const unsubscribe = mutation.subscribe(() => {})
    try {
      await mutation.mutate(
        { humanHelpEnabled: false },
        {
          onSuccess: (saved) =>
            client.setQueryData(useAgentSettings.getKey(), saved),
        },
      )
      read.resolve(on)
      await pendingRead
      expect(
        client.getQueryData<AgentSettings>(useAgentSettings.getKey()),
      ).toEqual(off)
    } finally {
      read.resolve(on)
      unsubscribe()
      client.clear()
    }
  })
})
