import { beforeEach, describe, expect, it } from 'bun:test'
import { AgentSessionStore } from '../../src/agent/agent-session-store'

describe('AgentSessionStore', () => {
  let store: AgentSessionStore

  beforeEach(() => {
    store = new AgentSessionStore()
  })

  describe('openSession', () => {
    it('opens a new session and tracks it', async () => {
      const session = await store.openSession(
        'agent-1',
        'session-1',
        '/home/user/project',
      )

      expect(session.sessionId).toBe('session-1')
      expect(session.agentId).toBe('agent-1')
      expect(session.refCount).toBe(1)
      expect(session.cwd).toBe('/home/user/project')
      expect(session.turnCount).toBe(0)
      expect(session.mode).toBe('default')
      expect(session.createdAt).toBeGreaterThan(0)
      expect(session.updatedAt).toBeGreaterThan(0)
      expect(session.createdAt).toBe(session.updatedAt)
    })

    it('increments ref count on re-open', async () => {
      await store.openSession('agent-1', 'session-1')
      const session = await store.openSession('agent-1', 'session-1')

      expect(session.refCount).toBe(2)
    })

    it('tracks sessions per agent independently', async () => {
      await store.openSession('agent-1', 'session-1')
      await store.openSession('agent-2', 'session-2')

      const agent1Sessions = await store.listSessions('agent-1')
      const agent2Sessions = await store.listSessions('agent-2')

      expect(agent1Sessions).toHaveLength(1)
      expect(agent2Sessions).toHaveLength(1)
      expect(agent1Sessions[0].sessionId).toBe('session-1')
      expect(agent2Sessions[0].sessionId).toBe('session-2')
    })
  })

  describe('closeSession', () => {
    it('decrements ref count on close; deletes at zero', async () => {
      await store.openSession('agent-1', 'session-1')

      const remaining = await store.closeSession('session-1')
      expect(remaining).toBe(0)

      const session = await store.getSessionMeta('session-1')
      expect(session).toBeNull()
    })

    it('returns remaining ref count after partial close', async () => {
      await store.openSession('agent-1', 'session-1')
      await store.openSession('agent-1', 'session-1')

      const remaining = await store.closeSession('session-1')
      expect(remaining).toBe(1)

      const session = await store.getSessionMeta('session-1')
      expect(session).not.toBeNull()
      expect(session?.refCount).toBe(1)
    })

    it('returns -1 for unknown session', async () => {
      const remaining = await store.closeSession('nonexistent')
      expect(remaining).toBe(-1)
    })
  })

  describe('listSessions', () => {
    it('lists sessions for an agent ordered by updatedAt descending', async () => {
      await store.openSession('agent-1', 'session-1')
      await Bun.sleep(1)
      await store.openSession('agent-1', 'session-2')
      await Bun.sleep(1)
      await store.openSession('agent-1', 'session-3')

      const sessions = await store.listSessions('agent-1')
      expect(sessions).toHaveLength(3)
      // Most recently created/updated first
      expect(sessions[0].sessionId).toBe('session-3')
      expect(sessions[1].sessionId).toBe('session-2')
      expect(sessions[2].sessionId).toBe('session-1')
    })

    it('returns empty array for agent with no sessions', async () => {
      const sessions = await store.listSessions('agent-1')
      expect(sessions).toEqual([])
    })

    it('searches sessions by title (case-insensitive)', async () => {
      await store.openSession('agent-1', 'session-1')
      await store.updateSessionMeta('session-1', { title: 'Fix Login Bug' })

      await store.openSession('agent-1', 'session-2')
      await store.updateSessionMeta('session-2', { title: 'Update README' })

      await store.openSession('agent-1', 'session-3')
      await store.updateSessionMeta('session-3', {
        title: 'Login Page Redesign',
      })

      const results = await store.listSessions('agent-1', { search: 'login' })
      expect(results).toHaveLength(2)
      const titles = results.map((s) => s.title)
      expect(titles).toContain('Fix Login Bug')
      expect(titles).toContain('Login Page Redesign')
    })

    it('returns empty when search matches nothing', async () => {
      await store.openSession('agent-1', 'session-1')
      await store.updateSessionMeta('session-1', { title: 'Fix Login Bug' })

      const results = await store.listSessions('agent-1', {
        search: 'xyznonexistent',
      })
      expect(results).toEqual([])
    })

    it('supports cursor-based pagination', async () => {
      // Create 5 sessions with distinct timestamps
      for (let i = 1; i <= 5; i++) {
        await store.openSession('agent-1', `session-${i}`)
        await Bun.sleep(1)
      }

      // Page 1: limit 2
      const page1 = await store.listSessions('agent-1', { limit: 2 })
      expect(page1).toHaveLength(2)
      expect(page1[0].sessionId).toBe('session-5')
      expect(page1[1].sessionId).toBe('session-4')

      // Page 2: cursor from last item of page 1
      const cursor = page1[1].sessionId
      const page2 = await store.listSessions('agent-1', { limit: 2, cursor })
      expect(page2).toHaveLength(2)
      expect(page2[0].sessionId).toBe('session-3')
      expect(page2[1].sessionId).toBe('session-2')

      // Page 3: remaining
      const cursor2 = page2[1].sessionId
      const page3 = await store.listSessions('agent-1', {
        limit: 2,
        cursor: cursor2,
      })
      expect(page3).toHaveLength(1)
      expect(page3[0].sessionId).toBe('session-1')
    })
  })

  describe('getSessionMeta', () => {
    it('returns session metadata', async () => {
      await store.openSession('agent-1', 'session-1', '/home/user')
      const session = await store.getSessionMeta('session-1')

      expect(session).not.toBeNull()
      expect(session?.sessionId).toBe('session-1')
      expect(session?.agentId).toBe('agent-1')
      expect(session?.cwd).toBe('/home/user')
    })

    it('returns null for unknown session', async () => {
      const session = await store.getSessionMeta('nonexistent')
      expect(session).toBeNull()
    })
  })

  describe('updateSessionMeta', () => {
    it('updates session metadata', async () => {
      await store.openSession('agent-1', 'session-1')
      const updated = await store.updateSessionMeta('session-1', {
        title: 'My Session',
        turnCount: 5,
        mode: 'code',
        model: 'gpt-4o',
        lastMessagePreview: 'Hello world',
      })

      expect(updated).not.toBeNull()
      expect(updated?.title).toBe('My Session')
      expect(updated?.turnCount).toBe(5)
      expect(updated?.mode).toBe('code')
      expect(updated?.model).toBe('gpt-4o')
      expect(updated?.lastMessagePreview).toBe('Hello world')
      await Bun.sleep(1)
      const afterUpdate = await store.updateSessionMeta('session-1', {
        title: 'My Session',
        turnCount: 5,
        mode: 'code',
        model: 'gpt-4o',
        lastMessagePreview: 'Hello world',
      })

      expect(afterUpdate?.updatedAt).toBeGreaterThan(afterUpdate?.createdAt)
    })

    it('returns null for unknown session', async () => {
      const result = await store.updateSessionMeta('nonexistent', {
        title: 'X',
      })
      expect(result).toBeNull()
    })

    it('preserves unmodified fields on partial update', async () => {
      await store.openSession('agent-1', 'session-1', '/home/user')
      await store.updateSessionMeta('session-1', { title: 'New Title' })

      const session = await store.getSessionMeta('session-1')
      expect(session?.title).toBe('New Title')
      expect(session?.cwd).toBe('/home/user')
      expect(session?.refCount).toBe(1)
    })

    it('supports meta object updates', async () => {
      await store.openSession('agent-1', 'session-1')
      await store.updateSessionMeta('session-1', {
        meta: { key1: 'value1', nested: { a: 1 } },
      })

      const session = await store.getSessionMeta('session-1')
      expect(session?.meta).toEqual({ key1: 'value1', nested: { a: 1 } })
    })

    it('sets lastMessageAt when updating lastMessagePreview', async () => {
      await store.openSession('agent-1', 'session-1')
      const before = Date.now()
      const updated = await store.updateSessionMeta('session-1', {
        lastMessagePreview: 'new message',
      })

      expect(updated?.lastMessageAt).toBeGreaterThanOrEqual(before)
      expect(updated?.lastMessageAt).toBeLessThanOrEqual(Date.now())
    })
  })
})
