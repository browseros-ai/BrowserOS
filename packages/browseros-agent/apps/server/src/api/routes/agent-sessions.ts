/**
 * @license
 * Copyright 2025 BrowserOS
 * SPDX-License-Identifier: AGPL-3.0-or-later
 */

import { randomUUID } from 'node:crypto'
import { Hono } from 'hono'
import type { AgentSessionStore } from '../../agent/agent-session-store'
import type { Env } from '../types'

export function createAgentSessionRoutes(deps: {
  sessionStore: AgentSessionStore
}) {
  const { sessionStore } = deps

  return new Hono<Env>()
    .post('/:agentId/sessions', async (c) => {
      const agentId = c.req.param('agentId')
      const body = await c.req.json().catch(() => ({}))
      const cwd =
        typeof body?.cwd === 'string' ? body.cwd.trim() || undefined : undefined

      const sessionId = randomUUID()
      const session = await sessionStore.openSession(agentId, sessionId, cwd)

      return c.json({ session }, 201)
    })
    .get('/:agentId/sessions', async (c) => {
      const agentId = c.req.param('agentId')
      const url = new URL(c.req.url)
      const search = url.searchParams.get('search') ?? undefined
      const cursor = url.searchParams.get('cursor') ?? undefined
      const limitRaw = url.searchParams.get('limit')
      const limit = limitRaw ? Number.parseInt(limitRaw, 10) : undefined
      const cappedLimit = Number.isFinite(limit) ? Math.min(limit, 200) : undefined

      const sessions = await sessionStore.listSessions(agentId, {
        search,
        cursor,
        limit: cappedLimit,
      })

      return c.json({ sessions })
    })
    .get('/:agentId/sessions/:sessionId', async (c) => {
      const agentId = c.req.param('agentId')
      const sessionId = c.req.param('sessionId')
      const session = await sessionStore.getSessionMeta(sessionId)
      if (!session || session.agentId !== agentId) return c.json({ error: 'Session not found' }, 404)
      return c.json({ session })
    })
    .patch('/:agentId/sessions/:sessionId', async (c) => {
      const agentId = c.req.param('agentId')
      const sessionId = c.req.param('sessionId')

      // Verify ownership before update
      const existing = await sessionStore.getSessionMeta(sessionId)
      if (!existing || existing.agentId !== agentId) return c.json({ error: 'Session not found' }, 404)

      const body = await c.req.json().catch(() => ({}))
      if (!body || typeof body !== 'object') {
        return c.json({ error: 'Invalid JSON body' }, 400)
      }

      const updates: Parameters<typeof sessionStore.updateSessionMeta>[1] = {}
      if (typeof body.title === 'string') updates.title = body.title
      if (Number.isInteger(body.turnCount) && body.turnCount >= 0) updates.turnCount = body.turnCount
      if (typeof body.lastMessagePreview === 'string')
        updates.lastMessagePreview = body.lastMessagePreview
      if (typeof body.lastMessageAt === 'number')
        updates.lastMessageAt = body.lastMessageAt
      if (typeof body.mode === 'string') updates.mode = body.mode
      if (typeof body.model === 'string') updates.model = body.model
      if (typeof body.meta === 'object' && body.meta !== null)
        updates.meta = body.meta

      if (Object.keys(updates).length === 0) {
        return c.json({ error: 'No editable fields supplied' }, 400)
      }

      const session = await sessionStore.updateSessionMeta(sessionId, updates)
      if (!session) return c.json({ error: 'Session not found' }, 404)
      return c.json({ session })
    })
    .delete('/:agentId/sessions/:sessionId', async (c) => {
      const agentId = c.req.param('agentId')
      const sessionId = c.req.param('sessionId')

      // Verify ownership before delete
      const existing = await sessionStore.getSessionMeta(sessionId)
      if (!existing || existing.agentId !== agentId) return c.json({ error: 'Session not found' }, 404)

      const refCount = await sessionStore.closeSession(sessionId)
      return c.json({ refCount })
    })
}
