/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, test } from 'bun:test'
import { Hono } from 'hono'
import { setMcpRequestHygieneMiddleware } from '../../src/routes/mcp/mcp-request-hygiene'

function buildTestApp() {
  return new Hono()
    .use('/mcp', setMcpRequestHygieneMiddleware)
    .all('/mcp', (c) => c.json({ ok: true }))
}

describe('setMcpRequestHygieneMiddleware', () => {
  test('accepts POST from a native-shape client', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ jsonrpc: '2.0', method: 'ping' }),
    })
    expect(res.status).toBe(200)
    expect(await res.json()).toEqual({ ok: true })
  })

  test('accepts GET without origin or sec-fetch headers', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', { method: 'GET' })
    expect(res.status).toBe(200)
  })

  test('rejects when origin header is present', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        origin: 'https://example.com',
      },
    })
    expect(res.status).toBe(403)
  })

  test('rejects when origin header is the literal string null', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        origin: 'null',
      },
    })
    expect(res.status).toBe(403)
  })

  test('rejects when origin is a chrome-extension origin', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        origin: 'chrome-extension://abcdefghijklmnop',
      },
    })
    expect(res.status).toBe(403)
  })

  test('rejects when sec-fetch-site is cross-site', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'sec-fetch-site': 'cross-site',
      },
    })
    expect(res.status).toBe(403)
  })

  test('rejects when sec-fetch-site is same-origin', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'sec-fetch-site': 'same-origin',
      },
    })
    expect(res.status).toBe(403)
  })

  test('rejects when sec-fetch-site is none', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        'sec-fetch-site': 'none',
      },
    })
    expect(res.status).toBe(403)
  })

  test('rejects write with content-type text/plain', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: { 'content-type': 'text/plain' },
      body: 'hi',
    })
    expect(res.status).toBe(415)
  })

  test('rejects write with content-type application/x-www-form-urlencoded', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: { 'content-type': 'application/x-www-form-urlencoded' },
      body: 'a=b',
    })
    expect(res.status).toBe(415)
  })

  test('rejects write with no content-type', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', { method: 'POST', body: 'x' })
    expect(res.status).toBe(415)
  })

  test('allows write with application/json plus a charset', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: { 'content-type': 'application/json; charset=utf-8' },
      body: '{}',
    })
    expect(res.status).toBe(200)
  })

  test('response body reveals no detail about the check', async () => {
    const app = buildTestApp()
    const res = await app.request('/mcp', {
      method: 'POST',
      headers: {
        'content-type': 'application/json',
        origin: 'https://example.com',
      },
    })
    const body = (await res.json()) as { error: string }
    expect(body.error).toBe('unsupported request')
    expect(body.error).not.toMatch(/origin/i)
    expect(body.error).not.toMatch(/sec-fetch/i)
  })
})
