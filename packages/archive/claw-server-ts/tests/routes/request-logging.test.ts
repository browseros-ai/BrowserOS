/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 *
 * Tests for the request-failure log middleware in src/server.ts.
 * Every >=400 response must produce exactly one structured
 * 'request failed' line (warn for 4xx, error for 5xx) regardless of
 * whether the failure was a router 404, a direct error response, or
 * an unhandled error resolved by `app.onError`; sub-400 traffic stays
 * unlogged so polling endpoints cannot flood the rotating log file.
 *
 * The thrown-error path runs on a fixture app wired like server.ts
 * (same middleware + an onError handler): the shared app's
 * route matcher is already built once any test file has fetched
 * through it, so throw-only routes cannot be mounted there.
 */

import { afterEach, beforeEach, describe, expect, spyOn, test } from 'bun:test'
import { Hono } from 'hono'
import { logger } from '../../src/lib/logger'
import { createServer, requestFailureLog } from '../../src/server'

const app = createServer()

let warnSpy: ReturnType<typeof spyOn<typeof logger, 'warn'>>
let errorSpy: ReturnType<typeof spyOn<typeof logger, 'error'>>

beforeEach(() => {
  warnSpy = spyOn(logger, 'warn')
  errorSpy = spyOn(logger, 'error')
})

afterEach(() => {
  warnSpy.mockRestore()
  errorSpy.mockRestore()
})

describe('request-failure logging on the live app', () => {
  test('successful responses log nothing', async () => {
    const res = await app.fetch(new Request('http://localhost/system/health'))
    expect(res.status).toBe(200)
    expect(warnSpy).not.toHaveBeenCalled()
    expect(errorSpy).not.toHaveBeenCalled()
  })

  test('router 404 logs one warn with method, path, status, duration', async () => {
    const res = await app.fetch(new Request('http://localhost/__no-such-route'))
    expect(res.status).toBe(404)
    expect(warnSpy).toHaveBeenCalledTimes(1)
    expect(errorSpy).not.toHaveBeenCalled()
    const [msg, fields] = warnSpy.mock.calls[0] ?? []
    expect(msg).toBe('request failed')
    expect(fields).toMatchObject({
      method: 'GET',
      path: '/__no-such-route',
      status: 404,
    })
    expect(fields?.durationMs).toBeGreaterThanOrEqual(0)
  })
})

describe('request-failure logging with thrown errors (fixture app)', () => {
  // Mirrors server.ts's composition so the middleware observes the
  // final 500 produced by the error handler.
  function fixtureApp(): Hono {
    const fx = new Hono()
    fx.onError((_err, c) => c.json({ error: 'internal error' }, 500))
    fx.use('*', requestFailureLog)
    fx.get('/boom', () => {
      throw new Error('boom')
    })
    fx.get('/direct', (c) => c.json({ error: 'gone' }, 410))
    fx.get('/ok', (c) => c.json({ ok: true }))
    return fx
  }

  test('unhandled error logs one error line with status 500', async () => {
    const res = await fixtureApp().fetch(new Request('http://localhost/boom'))
    expect(res.status).toBe(500)
    expect(errorSpy).toHaveBeenCalledTimes(1)
    expect(warnSpy).not.toHaveBeenCalled()
    const [msg, fields] = errorSpy.mock.calls[0] ?? []
    expect(msg).toBe('request failed')
    expect(fields).toMatchObject({
      method: 'GET',
      path: '/boom',
      status: 500,
    })
    expect(fields?.durationMs).toBeGreaterThanOrEqual(0)
  })

  test('direct 4xx JSON return logs one warn with its status', async () => {
    const res = await fixtureApp().fetch(new Request('http://localhost/direct'))
    expect(res.status).toBe(410)
    expect(warnSpy).toHaveBeenCalledTimes(1)
    expect(errorSpy).not.toHaveBeenCalled()
    expect(warnSpy.mock.calls[0]?.[1]).toMatchObject({
      method: 'GET',
      path: '/direct',
      status: 410,
    })
  })

  test('sub-400 responses log nothing', async () => {
    const res = await fixtureApp().fetch(new Request('http://localhost/ok'))
    expect(res.status).toBe(200)
    expect(warnSpy).not.toHaveBeenCalled()
    expect(errorSpy).not.toHaveBeenCalled()
  })
})
