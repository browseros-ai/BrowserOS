import { afterAll, describe, expect, it, mock } from 'bun:test'
import type { TelemetryState } from '@browseros/claw-api'
import { MutationObserver, QueryClient } from '@tanstack/react-query'
import * as api from '@/modules/api/client'

const enabled: TelemetryState = {
  distinctId: 'canonical-analytics-id',
  enabled: true,
  consent: true,
}
const disabled: TelemetryState = { ...enabled, enabled: false, consent: false }
const poll = Promise.withResolvers<TelemetryState>()
const pollStarted = Promise.withResolvers<void>()
const actualApiClient = api.apiClient

mock.module('@/modules/api/client', () => ({
  ...api,
  apiClient: async () =>
    Object.assign(await actualApiClient(), {
      getTelemetry: async () => {
        pollStarted.resolve()
        return poll.promise
      },
      updateTelemetry: async () => disabled,
    }),
}))

const { TELEMETRY_QUERY_KEY, useSetTelemetryConsent, useTelemetryState } =
  await import('./telemetry.hooks')

afterAll(() => mock.restore())

describe('telemetry consent ordering', () => {
  it('keeps a successful opt-out when an older telemetry poll resolves later', async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    client.setQueryData(TELEMETRY_QUERY_KEY, enabled)
    // Model a poll that read consent=true before PUT, but whose response arrives
    // after the toggle has successfully saved consent=false. The transport does
    // not consume AbortSignal, so cancellation must also discard its late result.
    const pendingPoll = client
      .fetchQuery({ ...useTelemetryState.getOptions(), staleTime: 0 })
      .catch(() => undefined)
    await pollStarted.promise
    const mutation = new MutationObserver(
      client,
      useSetTelemetryConsent.getOptions(),
    )
    const unsubscribe = mutation.subscribe(() => {})
    try {
      await mutation.mutate(
        { consent: false },
        {
          onSuccess: (state) => client.setQueryData(TELEMETRY_QUERY_KEY, state),
        },
      )
      expect(client.getQueryData<TelemetryState>(TELEMETRY_QUERY_KEY)).toEqual(
        disabled,
      )
      poll.resolve(enabled)
      await pendingPoll
      expect(client.getQueryData<TelemetryState>(TELEMETRY_QUERY_KEY)).toEqual(
        disabled,
      )
    } finally {
      poll.resolve(enabled)
      unsubscribe()
      client.clear()
    }
  })
})
