/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, mock, test } from 'bun:test'
import pkg from '../../package.json' with { type: 'json' }
import app, { createServer } from '../../src/server'
import { VERSION } from '../../src/version'

describe('system routes', () => {
  test('default server exposes system health', async () => {
    const res = await app.fetch(new Request('http://localhost/system/health'))

    expect(res.status).toBe(200)
    await expect(res.json()).resolves.toEqual({ status: 'ok' })
  })

  test('system version uses the shared package version', async () => {
    const res = await app.fetch(new Request('http://localhost/system/version'))

    expect(res.status).toBe(200)
    await expect(res.json()).resolves.toEqual({
      name: pkg.name,
      version: VERSION,
    })
    expect(VERSION).toBe(pkg.version)
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
