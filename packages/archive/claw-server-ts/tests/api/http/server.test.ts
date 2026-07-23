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
import type { BrowserSession } from '@browseros/browser-core/core/session'
import type { SessionDetail } from '@browseros/claw-api'
import { Hono } from 'hono'
import { createSessionHandlers } from '../../../src/api/http/handlers/sessions'
import { setBrowserSession } from '../../../src/lib/browser-session'
import { logger } from '../../../src/lib/logger'
import { identityService } from '../../../src/lib/mcp-session'
import {
  resetAuditDbForTesting,
  setAuditDbForTesting,
} from '../../../src/modules/db/db'
import { createServer, requestFailureLog } from '../../../src/server'
import { recordToolDispatch } from '../../../src/services/audit-log'
import { defaultHttpHandlers } from './fixtures'

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

describe('mounted HTTP middleware', () => {
  test('rejects hostile recording origins before CORS handles preflight', async () => {
    const app = createServer({ httpHandlers: defaultHttpHandlers() })
    const response = await app.request(
      'http://127.0.0.1/api/v1/recordings/events',
      {
        method: 'OPTIONS',
        headers: {
          origin: 'https://attacker.example',
          'access-control-request-method': 'POST',
          'access-control-request-headers':
            'content-type,x-recording-tab-id,x-recording-document-id,x-recording-batch-id',
        },
      },
    )
    expect(response.status).toBe(403)
    expect(response.headers.get('access-control-allow-origin')).toBeNull()
    expect(await response.json()).toMatchObject({ code: 'forbidden' })
  })

  test('keeps permissive CORS on normal API requests', async () => {
    const response = await createServer({
      httpHandlers: defaultHttpHandlers(),
    }).request('http://127.0.0.1/api/v1/system', {
      headers: { origin: 'https://example.com' },
    })
    expect(response.status).toBe(200)
    expect(response.headers.get('access-control-allow-origin')).toBe('*')
  })

  test('uses the canonical error shape for unexpected handler failures', async () => {
    const handlers = defaultHttpHandlers()
    handlers.sessions = createSessionHandlers({
      listSessions: () => {
        throw new Error('database unavailable')
      },
      getSession: () => null,
      getSessionState: () => 'missing',
      cancelSession: () => 0,
    })
    const response = await createServer({ httpHandlers: handlers }).request(
      'http://localhost/api/v1/sessions',
    )
    expect(response.status).toBe(500)
    expect(await response.json()).toMatchObject({
      code: 'internal_error',
      message: 'internal server error',
      requestId: expect.any(String),
    })
  })

  test('copies the server-minted request id into canonical errors', async () => {
    const handlers = defaultHttpHandlers()
    handlers.sessions = createSessionHandlers({
      listSessions: () => ({ items: [] }),
      getSession: () => null,
      getSessionState: () => 'missing',
      cancelSession: () => 0,
    })
    const response = await createServer({ httpHandlers: handlers }).request(
      'http://localhost/api/v1/sessions/missing',
    )
    const body = (await response.json()) as { requestId?: string }
    expect(response.status).toBe(404)
    expect(body.requestId).toBeString()
    expect(response.headers.get('x-request-id')).toBe(body.requestId)
  })
})

describe('mounted production HTTP handlers', () => {
  beforeEach(() => {
    setAuditDbForTesting()
    identityService.clear()
  })

  afterEach(() => {
    identityService.clear()
    setBrowserSession(null)
    resetAuditDbForTesting()
  })

  test('keeps connected sessions visible when browser reconciliation fails', async () => {
    identityService.registerInitialize({
      sessionId: 'session-connected',
      clientInfo: { name: 'codex', version: '1.0.0', title: 'Codex CLI' },
    })
    recordToolDispatch({
      agentId: 'codex-session-connected',
      slug: 'codex',
      agentLabel: 'Codex CLI',
      sessionId: 'session-connected',
      toolName: 'name_session',
      pageId: null,
      tabId: null,
      targetId: null,
      url: null,
      title: null,
      rawArgs: { name: 'connected session' },
      durationMs: 1,
      result: { isError: false, content: [], structuredContent: {} },
    })
    setBrowserSession({
      pages: {
        list: async () => {
          throw new Error('CDP unavailable')
        },
      },
    } as unknown as BrowserSession)
    const consoleError = spyOn(console, 'error').mockImplementation(() => {})
    try {
      const response = await createServer().request(
        'http://localhost/api/v1/sessions?status=live',
      )
      expect(response.status).toBe(200)
      expect(await response.json()).toMatchObject({
        items: [
          {
            sessionId: 'session-connected',
            status: 'live',
            live: { state: 'idle', browserTabs: [] },
          },
        ],
      })
      expect(
        consoleError.mock.calls.some(([line]) =>
          String(line).includes(
            'live session browser reconciliation unavailable',
          ),
        ),
      ).toBe(true)
    } finally {
      consoleError.mockRestore()
    }
  })

  test('maps persisted tasks without leaking internal identities or nulls', async () => {
    recordToolDispatch({
      agentId: 'codex-generated-name',
      slug: 'codex',
      agentLabel: 'Codex',
      sessionId: 'session-1',
      toolName: 'snapshot',
      pageId: 7,
      targetId: 'target-7',
      url: null,
      title: null,
      rawArgs: {},
      durationMs: 5,
      result: { isError: false, content: [], structuredContent: {} },
    })

    const response = await createServer().request(
      'http://localhost/api/v1/sessions/session-1',
    )
    expect(response.status).toBe(200)
    const body = (await response.json()) as SessionDetail
    expect(body.session).toMatchObject({
      sessionId: 'session-1',
      slug: 'codex',
      label: 'Codex',
      status: 'live',
    })
    expect(body.dispatches[0]).toMatchObject({
      dispatchId: 1,
      pageId: 7,
      targetId: 'target-7',
    })
    expect(body.dispatches[0]).not.toHaveProperty('screenshotId')
    expect(JSON.stringify(body)).not.toContain('agentId')
    expect(JSON.stringify(body)).not.toContain(':null')
  })
})
