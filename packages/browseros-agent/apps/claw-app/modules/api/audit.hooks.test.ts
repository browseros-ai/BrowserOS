import { beforeEach, describe, expect, it, mock } from 'bun:test'
import type { SessionList } from '@browseros/claw-api'
import * as _client from './client'

const response: SessionList = { items: [] }
const listSessions = mock(async () => response)

mock.module('./client', () => ({
  ..._client,
  apiClient: async () => ({ listSessions }),
}))

const { useLiveSessions } = await import('./audit.hooks')

beforeEach(() => {
  listSessions.mockClear()
})

describe('useLiveSessions', () => {
  it('polls a dedicated complete live-session snapshot', async () => {
    expect(Array.from(useLiveSessions.getKey())).toEqual([
      'api',
      'sessions',
      'live',
    ])
    expect(useLiveSessions.getOptions().refetchInterval).toBe(1500)

    expect(await useLiveSessions.fetcher(undefined)).toBe(response)
    expect(listSessions).toHaveBeenCalledWith({ status: 'live' })
  })
})
