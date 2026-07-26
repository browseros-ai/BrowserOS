import { afterAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { CockpitStats } from '@browseros/claw-api'
import { QueryClient, QueryObserver } from '@tanstack/react-query'
import * as _client from './client'

const response: CockpitStats = {
  hasMeasuredStats: true,
  allTime: {
    browserClawTokenEstimate: 2_000,
    screenshotFirstTokenEstimate: 1_000,
    rawTokenSavingsEstimate: -1_000,
    humanTimeSavedMs: 90_000,
    sessionCount: 2,
    toolCallCount: 8,
  },
  last30Days: {
    browserClawTokenEstimate: 0,
    screenshotFirstTokenEstimate: 0,
    rawTokenSavingsEstimate: 0,
    humanTimeSavedMs: 0,
    sessionCount: 0,
    toolCallCount: 0,
  },
  last7Days: {
    browserClawTokenEstimate: 0,
    screenshotFirstTokenEstimate: 0,
    rawTokenSavingsEstimate: 0,
    humanTimeSavedMs: 0,
    sessionCount: 0,
    toolCallCount: 0,
  },
}

const getCockpitStats = mock(async () => response)
const actualApiClient = _client.apiClient

mock.module('./client', () => ({
  ..._client,
  apiClient: async () =>
    Object.assign(await actualApiClient(), { getCockpitStats }),
}))

const { useCockpitStats } = await import('./cockpit.hooks')

afterAll(() => mock.restore())

beforeEach(() => {
  getCockpitStats.mockClear()
})

describe('useCockpitStats', () => {
  it('uses a stable API key, polls, and delegates without clamping', async () => {
    expect(Array.from(useCockpitStats.getKey())).toEqual([
      'api',
      'cockpit',
      'stats',
    ])
    expect(useCockpitStats.getOptions().refetchInterval).toBe(3000)

    const result = await useCockpitStats.fetcher(undefined)

    expect(result).toBe(response)
    expect(result.allTime.rawTokenSavingsEstimate).toBe(-1_000)
    expect(getCockpitStats).toHaveBeenCalledTimes(1)
  })

  it('does not fetch when the query is gated off', async () => {
    const client = new QueryClient({
      defaultOptions: { queries: { retry: false } },
    })
    const observer = new QueryObserver(client, {
      ...useCockpitStats.getOptions(),
      enabled: false,
    })
    const unsubscribe = observer.subscribe(() => {})

    await Promise.resolve()

    expect(observer.getCurrentResult().fetchStatus).toBe('idle')
    expect(getCockpitStats).not.toHaveBeenCalled()
    unsubscribe()
    client.clear()
  })
})
