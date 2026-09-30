import { afterAll, beforeEach, describe, expect, it, mock } from 'bun:test'
import type { SessionList, SessionScreenshotList } from '@browseros/claw-api'
import * as _client from './client'

const response: SessionList = { items: [] }
const screenshotResponse: SessionScreenshotList = {
  items: [{ screenshotId: 17, capturedAt: 123, toolName: 'act' }],
}
const listSessions = mock(async () => response)
const listSessionScreenshots = mock(async () => screenshotResponse)
const actualApiClient = _client.apiClient

mock.module('./client', () => ({
  ..._client,
  apiClient: async () =>
    Object.assign(await actualApiClient(), {
      listSessions,
      listSessionScreenshots,
    }),
}))

const {
  sessionPreviewUrl,
  taskScreenshotUrl,
  useLiveSessions,
  useSessionScreenshots,
} = await import('./audit.hooks')

afterAll(() => mock.restore())

beforeEach(() => {
  listSessions.mockClear()
  listSessionScreenshots.mockClear()
})

describe('session visual resources', () => {
  it('builds live and historical URLs from session-owned identities', () => {
    expect(
      sessionPreviewUrl('session / one', 456, 'http://127.0.0.1:9200'),
    ).toBe(
      'http://127.0.0.1:9200/api/v1/sessions/session%20%2F%20one/preview?refresh=456',
    )
    expect(
      taskScreenshotUrl('session / one', 17, 'http://127.0.0.1:9200'),
    ).toBe(
      'http://127.0.0.1:9200/api/v1/sessions/session%20%2F%20one/screenshots/17',
    )
  })

  it('exposes ordered screenshot metadata by session, polling opt-in', async () => {
    expect(Array.from(useSessionScreenshots.getKey())).toEqual([
      'api',
      'session',
      'screenshots',
    ])
    // No factory-level polling: callers opt in only while the session is live.
    expect(
      useSessionScreenshots.getOptions({ sessionId: 'session-1' })
        .refetchInterval,
    ).toBeUndefined()
    expect(
      await useSessionScreenshots.fetcher({ sessionId: 'session-1' }),
    ).toBe(screenshotResponse)
    expect(listSessionScreenshots).toHaveBeenCalledWith({
      sessionId: 'session-1',
    })
  })
})

describe('useLiveSessions', () => {
  it('exposes a dedicated complete live-session snapshot, polling opt-in', async () => {
    expect(Array.from(useLiveSessions.getKey())).toEqual([
      'api',
      'sessions',
      'live',
    ])
    // No factory-level polling: cockpit.data.ts opts in to the interval.
    expect(useLiveSessions.getOptions().refetchInterval).toBeUndefined()
    expect(
      useLiveSessions.getOptions().refetchIntervalInBackground,
    ).toBeUndefined()

    expect(await useLiveSessions.fetcher(undefined)).toBe(response)
    expect(listSessions).toHaveBeenCalledWith({ status: 'live' })
  })
})
