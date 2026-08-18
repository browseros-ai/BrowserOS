import { describe, expect, it } from 'bun:test'
import { Hono } from 'hono'
import type { Env } from '../../src/api/types'
import {
  isTrustedAppOrigin,
  requireTrustedAppOrigin,
} from '../../src/api/utils/request-auth'

const localServer = {
  server: {
    requestIP: () => ({ address: '127.0.0.1' }),
  },
} as never

describe('request auth', () => {
  it('accepts loopback and extension origins', () => {
    expect(isTrustedAppOrigin('http://127.0.0.1:9105')).toBe(true)
    expect(isTrustedAppOrigin('http://localhost:3000')).toBe(true)
    expect(isTrustedAppOrigin('chrome-extension://browseros')).toBe(true)
    expect(isTrustedAppOrigin('moz-extension://browseros')).toBe(true)
  })

  it('rejects missing and untrusted origins', () => {
    expect(isTrustedAppOrigin(undefined)).toBe(false)
    expect(isTrustedAppOrigin('https://example.com')).toBe(false)
    expect(isTrustedAppOrigin('file:///tmp/app.html')).toBe(false)
  })

  it('blocks requests from untrusted origins', async () => {
    const app = new Hono<Env>()
      .use('/*', requireTrustedAppOrigin())
      .get('/agents/status', (c) => c.json({ ok: true }))

    const res = await app.request('http://localhost/agents/status', {
      headers: { Origin: 'https://evil.example' },
    })

    expect(res.status).toBe(403)
    expect(await res.json()).toEqual({ error: 'Forbidden' })
  })

  it('allows requests from trusted origins', async () => {
    const app = new Hono<Env>()
      .use('/*', requireTrustedAppOrigin())
      .get('/agents/status', (c) => c.json({ ok: true }))

    const res = await app.request(
      'http://localhost/agents/status',
      {
        headers: {
          Host: 'localhost',
          Origin: 'chrome-extension://browseros',
        },
      },
      localServer,
    )

    expect(res.status).toBe(200)
    expect(await res.json()).toEqual({ ok: true })
  })
})
