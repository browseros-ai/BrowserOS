/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { Hono } from 'hono'
import { createAssistantSessionRoutes } from '../../../src/api/routes/assistant-sessions'

describe('createAssistantSessionRoutes', () => {
  function createApp() {
    return new Hono().route('/assistant', createAssistantSessionRoutes())
  }

  it('GET /sessions returns 501 Not Implemented', async () => {
    const app = createApp()
    const res = await app.request('/assistant/sessions')
    expect(res.status).toBe(501)
    const body = await res.json()
    expect(body.error).toMatch(/not implemented/i)
  })

  it('POST /sessions returns 501 Not Implemented', async () => {
    const app = createApp()
    const res = await app.request('/assistant/sessions', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({}),
    })
    expect(res.status).toBe(501)
  })

  it('GET /sessions/:id returns 501 Not Implemented', async () => {
    const app = createApp()
    const res = await app.request('/assistant/sessions/some-id')
    expect(res.status).toBe(501)
  })

  it('PATCH /sessions/:id returns 501 Not Implemented', async () => {
    const app = createApp()
    const res = await app.request('/assistant/sessions/some-id', {
      method: 'PATCH',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ title: 'x' }),
    })
    expect(res.status).toBe(501)
  })

  it('DELETE /sessions/:id returns 501 Not Implemented', async () => {
    const app = createApp()
    const res = await app.request('/assistant/sessions/some-id', {
      method: 'DELETE',
    })
    expect(res.status).toBe(501)
  })
})
