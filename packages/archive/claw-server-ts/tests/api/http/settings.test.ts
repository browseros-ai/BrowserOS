import { describe, expect, mock, test } from 'bun:test'
import { createSettingsHandlers } from '../../../src/api/http/handlers/settings'
import { httpTestApp, telemetry } from './fixtures'

describe('settings HTTP handlers', () => {
  test('reads and updates telemetry consent', async () => {
    const updateTelemetry = mock((consent: boolean) => ({
      ...telemetry,
      enabled: consent,
      consent,
    }))
    const app = httpTestApp({
      settings: createSettingsHandlers({
        getTelemetry: () => telemetry,
        updateTelemetry,
      }),
    })

    const read = await app.request('/api/v1/settings/telemetry')
    expect(read.status).toBe(200)
    expect(await read.json()).toEqual(telemetry)

    const update = await app.request('/api/v1/settings/telemetry', {
      method: 'PUT',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ consent: false }),
    })
    expect(update.status).toBe(200)
    expect(await update.json()).toEqual({
      ...telemetry,
      enabled: false,
      consent: false,
    })
    expect(updateTelemetry).toHaveBeenCalledWith(false)
  })

  test('rejects malformed and non-boolean consent bodies', async () => {
    const app = httpTestApp()
    for (const body of ['{', JSON.stringify({ consent: 'yes' })]) {
      const response = await app.request('/api/v1/settings/telemetry', {
        method: 'PUT',
        headers: { 'content-type': 'application/json' },
        body,
      })
      expect(response.status).toBe(400)
      expect(await response.json()).toMatchObject({
        code: 'invalid_request',
        message: 'consent must be a boolean',
      })
    }
  })
})
