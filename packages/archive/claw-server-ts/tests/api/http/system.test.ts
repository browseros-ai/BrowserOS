/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, mock, test } from 'bun:test'
import { createServer } from '../../../src/server'
import { httpTestApp, system } from './fixtures'

const app = createServer()

describe('system routes', () => {
  test('default server exposes system health', async () => {
    const res = await app.fetch(new Request('http://localhost/system/health'))

    expect(res.status).toBe(200)
    await expect(res.json()).resolves.toEqual({ status: 'ok' })
  })

  test('system shutdown responds before invoking the shutdown hook', async () => {
    const onShutdown = mock(() => {})
    const server = createServer({ onShutdown })

    const res = await server.fetch(
      new Request('http://localhost/system/shutdown', { method: 'POST' }),
    )

    expect(res.status).toBe(200)
    await expect(res.json()).resolves.toEqual({ status: 'ok' })
    expect(onShutdown).not.toHaveBeenCalled()

    await new Promise<void>((resolve) => setImmediate(resolve))

    expect(onShutdown).toHaveBeenCalledTimes(1)
  })

  test('serves system information through the centralized route table', async () => {
    const response = await httpTestApp().request('/api/v1/system')
    expect(response.status).toBe(200)
    expect(await response.json()).toEqual(system)
  })

  test('matches the frozen HTTP route table', () => {
    const routes = httpTestApp().routes.map(
      ({ method, path }) => `${method} ${path}`,
    )
    expect([...new Set(routes)]).toEqual([
      'GET /system/health',
      'POST /system/shutdown',
      'GET /api/v1/system',
      'GET /api/v1/settings/telemetry',
      'PUT /api/v1/settings/telemetry',
      'GET /api/v1/sessions',
      'GET /api/v1/sessions/:sessionId',
      'POST /api/v1/sessions/:sessionId/cancel',
      'GET /api/v1/sessions/:sessionId/recording',
      'GET /api/v1/sessions/:sessionId/recording/events',
      'POST /api/v1/recordings/events',
      'GET /api/v1/sessions/:sessionId/preview',
      'GET /api/v1/sessions/:sessionId/screenshots',
      'GET /api/v1/sessions/:sessionId/screenshots/:screenshotId',
      'GET /api/v1/connections',
      'PUT /api/v1/connections/:harness',
      'DELETE /api/v1/connections/:harness',
    ])
  })
})
