/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { describe, expect, it } from 'bun:test'
import { Hono } from 'hono'
import { AgentSessionStore } from '../../../src/agent/agent-session-store'
import { createAgentSessionRoutes } from '../../../src/api/routes/agent-sessions'

describe('createAgentSessionRoutes', () => {
  function createApp(store?: AgentSessionStore) {
    const s = store ?? new AgentSessionStore()
    return new Hono().route(
      '/agents',
      createAgentSessionRoutes({ sessionStore: s }),
    )
  }

  describe('POST /:agentId/sessions', () => {
    it('creates a session and returns it', async () => {
      const app = createApp()
      const res = await app.request('/agents/agent-1/sessions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ cwd: '/tmp/work' }),
      })

      expect(res.status).toBe(201)
      const body = await res.json()
      expect(body.session).toMatchObject({
        agentId: 'agent-1',
        refCount: 1,
        cwd: '/tmp/work',
        mode: 'default',
        turnCount: 0,
      })
      expect(body.session.sessionId).toBeTruthy()
    })

    it('creates a session without cwd', async () => {
      const app = createApp()
      const res = await app.request('/agents/agent-1/sessions', {
        method: 'POST',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({}),
      })

      expect(res.status).toBe(201)
      const body = await res.json()
      expect(body.session.cwd).toBeNull()
    })
  })

  describe('GET /:agentId/sessions', () => {
    it('lists sessions for an agent', async () => {
      const store = new AgentSessionStore()
      await store.openSession('agent-1', 's1', '/tmp')
      await store.openSession('agent-1', 's2', '/home')
      // Different agent — should not appear
      await store.openSession('agent-2', 's3')

      const app = createApp(store)
      const res = await app.request('/agents/agent-1/sessions')

      expect(res.status).toBe(200)
      const body = await res.json()
      expect(body.sessions).toHaveLength(2)
      expect(body.sessions.map((s: { sessionId: string }) => s.sessionId)).toEqual(
        expect.arrayContaining(['s1', 's2']),
      )
    })

    it('supports search filtering', async () => {
      const store = new AgentSessionStore()
      await store.openSession('agent-1', 's1')
      await store.updateSessionMeta('s1', { title: 'Bug fix session' })
      await store.openSession('agent-1', 's2')
      await store.updateSessionMeta('s2', { title: 'Feature work' })

      const app = createApp(store)
      const res = await app.request(
        '/agents/agent-1/sessions?search=bug+fix',
      )

      expect(res.status).toBe(200)
      const body = await res.json()
      expect(body.sessions).toHaveLength(1)
      expect(body.sessions[0].sessionId).toBe('s1')
    })

    it('supports cursor pagination', async () => {
      const store = new AgentSessionStore()
      await store.openSession('agent-1', 's1')
      await store.openSession('agent-1', 's2')
      await store.openSession('agent-1', 's3')

      const app = createApp(store)
      const res = await app.request(
        '/agents/agent-1/sessions?limit=2&cursor=s2',
      )

      expect(res.status).toBe(200)
      const body = await res.json()
      expect(body.sessions.length).toBeLessThanOrEqual(2)
    })

    it('returns empty list for agent with no sessions', async () => {
      const app = createApp()
      const res = await app.request('/agents/agent-1/sessions')

      expect(res.status).toBe(200)
      const body = await res.json()
      expect(body.sessions).toEqual([])
    })
  })

  describe('GET /:agentId/sessions/:sessionId', () => {
    it('returns a session by id', async () => {
      const store = new AgentSessionStore()
      await store.openSession('agent-1', 's1', '/tmp')

      const app = createApp(store)
      const res = await app.request('/agents/agent-1/sessions/s1')

      expect(res.status).toBe(200)
      const body = await res.json()
      expect(body.session).toMatchObject({
        sessionId: 's1',
        agentId: 'agent-1',
        cwd: '/tmp',
      })
    })

    it('returns 404 for unknown session', async () => {
      const app = createApp()
      const res = await app.request('/agents/agent-1/sessions/nope')

      expect(res.status).toBe(404)
    })
  })

  describe('PATCH /:agentId/sessions/:sessionId', () => {
    it('updates session metadata', async () => {
      const store = new AgentSessionStore()
      await store.openSession('agent-1', 's1')

      const app = createApp(store)
      const res = await app.request('/agents/agent-1/sessions/s1', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({
          title: 'My session',
          turnCount: 5,
          lastMessagePreview: 'hello',
        }),
      })

      expect(res.status).toBe(200)
      const body = await res.json()
      expect(body.session).toMatchObject({
        sessionId: 's1',
        title: 'My session',
        turnCount: 5,
        lastMessagePreview: 'hello',
        lastMessageAt: expect.any(Number),
      })
    })

    it('returns 404 for unknown session', async () => {
      const app = createApp()
      const res = await app.request('/agents/agent-1/sessions/nope', {
        method: 'PATCH',
        headers: { 'Content-Type': 'application/json' },
        body: JSON.stringify({ title: 'x' }),
      })

      expect(res.status).toBe(404)
    })
  })

  describe('DELETE /:agentId/sessions/:sessionId', () => {
    it('closes a session (ref-count decrement)', async () => {
      const store = new AgentSessionStore()
      await store.openSession('agent-1', 's1')
      // Bump refCount
      await store.openSession('agent-1', 's1')

      const app = createApp(store)
      const res = await app.request('/agents/agent-1/sessions/s1', {
        method: 'DELETE',
      })

      expect(res.status).toBe(200)
      const body = await res.json()
      expect(body.refCount).toBe(1)
    })

    it('fully deletes session when ref-count reaches zero', async () => {
      const store = new AgentSessionStore()
      await store.openSession('agent-1', 's1')

      const app = createApp(store)
      const res = await app.request('/agents/agent-1/sessions/s1', {
        method: 'DELETE',
      })

      expect(res.status).toBe(200)
      const body = await res.json()
      expect(body.refCount).toBe(0)

      // Verify it's gone
      const get = await app.request('/agents/agent-1/sessions/s1')
      expect(get.status).toBe(404)
    })

    it('returns 404 for unknown session', async () => {
      const app = createApp()
      const res = await app.request('/agents/agent-1/sessions/nope', {
        method: 'DELETE',
      })

      expect(res.status).toBe(404)
    })
  })
})
