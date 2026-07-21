import { describe, expect, mock, test } from 'bun:test'
import { createScreenshotHandlers } from '../../../src/api/http/handlers/screenshots'
import { httpTestApp } from './fixtures'

describe('screenshot HTTP handlers', () => {
  test('lists session screenshots and serves immutable JPEG bytes', async () => {
    const app = httpTestApp()
    const list = await app.request('/api/v1/sessions/session-live/screenshots')
    expect(list.status).toBe(200)
    expect(await list.json()).toEqual({
      items: [{ screenshotId: 1, capturedAt: 100, toolName: 'snapshot' }],
    })

    const screenshot = await app.request(
      '/api/v1/sessions/session-live/screenshots/1',
    )
    expect(screenshot.status).toBe(200)
    expect(screenshot.headers.get('content-type')).toBe('image/jpeg')
    expect(screenshot.headers.get('cache-control')).toBe(
      'public, max-age=31536000, immutable',
    )
  })

  test('validates positive screenshot ids before reading storage', async () => {
    const getSessionScreenshot = mock(async () => null)
    const app = httpTestApp({
      screenshots: createScreenshotHandlers({
        listSessionScreenshots: () => ({ items: [] }),
        getSessionScreenshot,
      }),
    })
    for (const id of ['0', '-1', '1.5', 'nope']) {
      const response = await app.request(
        `/api/v1/sessions/session-live/screenshots/${id}`,
      )
      expect(response.status).toBe(400)
      expect(await response.json()).toMatchObject({ code: 'invalid_request' })
    }
    expect(getSessionScreenshot).not.toHaveBeenCalled()
  })

  test('distinguishes an unknown session list from a missing screenshot', async () => {
    const app = httpTestApp({
      screenshots: createScreenshotHandlers({
        listSessionScreenshots: () => null,
        getSessionScreenshot: () => null,
      }),
    })
    const list = await app.request('/api/v1/sessions/missing/screenshots')
    expect(list.status).toBe(404)
    expect(await list.json()).toMatchObject({ code: 'session_not_found' })

    const read = await app.request('/api/v1/sessions/missing/screenshots/1')
    expect(read.status).toBe(404)
    expect(await read.json()).toMatchObject({ code: 'screenshot_not_found' })
  })
})
