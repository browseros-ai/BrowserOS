import { describe, expect, test } from 'bun:test'
import { createReplayHandlers } from '../../../src/api/http/handlers/replay'
import { httpTestApp, recording } from './fixtures'

describe('replay HTTP handlers', () => {
  test('returns metadata for a known session with no recording data', async () => {
    const response = await httpTestApp({
      replay: createReplayHandlers({
        getRecording: () => recording,
        downloadRecordingEvents: async () => '',
      }),
    }).request('/api/v1/sessions/session-live/recording')
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual(recording)
  })

  test('preserves the NDJSON content type and trailing newline', async () => {
    const response = await httpTestApp({
      replay: createReplayHandlers({
        getRecording: () => recording,
        downloadRecordingEvents: async () => '{"ts":1}\n',
      }),
    }).request('/api/v1/sessions/session-live/recording/events')
    expect(response.status).toBe(200)
    expect(response.headers.get('content-type')).toContain(
      'application/x-ndjson',
    )
    expect(await response.text()).toBe('{"ts":1}\n')
  })

  test('returns session_not_found for unknown metadata and event streams', async () => {
    const app = httpTestApp({
      replay: createReplayHandlers({
        getRecording: () => null,
        downloadRecordingEvents: async () => null,
      }),
    })
    for (const path of [
      '/api/v1/sessions/missing/recording',
      '/api/v1/sessions/missing/recording/events',
    ]) {
      const response = await app.request(path)
      expect(response.status).toBe(404)
      expect(await response.json()).toMatchObject({ code: 'session_not_found' })
    }
  })
})
