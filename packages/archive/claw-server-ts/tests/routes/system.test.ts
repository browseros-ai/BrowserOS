/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, mock, test } from 'bun:test'
import { createServer } from '../../src/server'

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
})
